//! The data-rep extension registry: the server-side sibling to [`CanvasRegistry`]
//! that holds the in-tree extensions and dispatches their MCP tool calls.
//!
//! The per-extension MCP registration path keeps the SINGLE `/mcp` transport and
//! the single [`SceneMcp`](crate::mcp::SceneMcp) facade: `tools/list` becomes the
//! core object tools PLUS, for each extension, its `mcp_tools()` namespaced
//! `ext_<name>_<tool>`; a call to such a name dispatches here. There is NO second
//! transport and NO second op-apply — an author runs `export()` -> `ObjectOp`s ->
//! the SAME [`ActorHandle::apply_op`] funnel the core write tools use.
//!
//! rmcp's `#[tool_router]` is compile-time, so in-tree extensions are composed at
//! build time ([`ExtensionRegistry::with_builtins`]); a runtime WASM-component
//! plugin system is a later option, not built now (no caller needs it).
//!
//! DOMAIN MODEL PERSISTENCE: the model is the source of truth and rides the
//! per-extension ROOT object's `meta[extModel]` blob, so it flows through the
//! existing op-apply/sync/storage path — the server stays stateless and there is
//! no sidecar second-source-of-truth. The registry loads the model from that blob
//! (or `empty_model()` when no root exists yet), authors, then re-exports.
//!
//! INCREMENTAL RE-EXPORT: re-export with the stable id allocator reuses object ids
//! (keyed off `meta[extKey]`). The registry DIFFS the desired object set against
//! the current scene so an edit emits minimal property ops (transform/text/style/
//! meta/...) for changed objects, `InsertObject` only for new ones, and `Delete`
//! for removed ones — never delete-and-reinsert. Geometry keys are stable (same
//! domain key => same rect dims), so no `EditGeometry` is emitted, preserving the
//! transform-only / zero-rebake bar.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use shape_extension_contract::{
    Extension, IdOrderAlloc, META_DOMAIN_KEY, META_EXT_KEY, META_MODEL_KEY,
};
use shape_scene_core::object::op::FieldEdit;
use shape_scene_core::object::{Object, ObjectOp, ObjectScene};

use crate::canvas_actor::{ActorHandle, ApplyResult};

/// The advertised namespace separator: a tool is `ext_<name>_<tool>` so it never
/// collides with a core object tool.
const NS_PREFIX: &str = "ext_";

/// Holds the boxed in-tree extensions. Cloning shares the handles (the extensions
/// are zero-sized stateless dispatchers), so the registry is cheap to clone per
/// MCP session like [`CanvasRegistry`].
#[derive(Clone)]
pub struct ExtensionRegistry {
    extensions: std::sync::Arc<Vec<Box<dyn Extension>>>,
}

/// One advertised extension tool, namespaced and self-describing — the row the
/// MCP `tools/list` adds on top of the core object tools.
pub struct ExtToolDef {
    /// The advertised, namespaced name: `ext_<name>_<tool>`.
    pub name: String,
    pub description: &'static str,
    /// The tool's light input schema (the extension's `McpToolMeta.schema`).
    pub schema: Value,
}

impl ExtensionRegistry {
    /// The in-tree extension set composed at build time: todo-kanban (layout-shaped)
    /// + structure-diagram (relational-shaped). Adding an extension is one line here.
    pub fn with_builtins() -> Self {
        let extensions: Vec<Box<dyn Extension>> = vec![
            Box::new(shape_ext_todo_kanban::KanbanExtension),
            Box::new(shape_ext_structure_diagram::DiagramExtension),
        ];
        ExtensionRegistry { extensions: std::sync::Arc::new(extensions) }
    }

    /// Construct from an explicit extension list (tests / future hosts).
    pub fn from_extensions(extensions: Vec<Box<dyn Extension>>) -> Self {
        ExtensionRegistry { extensions: std::sync::Arc::new(extensions) }
    }

    /// Every extension's tools, namespaced `ext_<name>_<tool>`, in registration
    /// order — appended to the core object tools by [`SceneMcp`](crate::mcp::SceneMcp).
    pub fn tool_defs(&self) -> Vec<ExtToolDef> {
        let mut out = Vec::new();
        for ext in self.extensions.iter() {
            for tool in ext.mcp_tools() {
                out.push(ExtToolDef {
                    name: format!("{NS_PREFIX}{}_{}", ext.name(), tool.name),
                    description: tool.description,
                    schema: tool.schema,
                });
            }
        }
        out
    }

    /// True iff `tool_name` is an extension-namespaced tool this registry serves.
    pub fn handles(&self, tool_name: &str) -> bool {
        self.split(tool_name).is_some()
    }

    /// Split `ext_<name>_<tool>` into `(extension, bare tool)`, longest extension
    /// match first (so an extension name never shadows a tool name with `_`).
    fn split<'a>(&self, tool_name: &'a str) -> Option<(&dyn Extension, &'a str)> {
        let rest = tool_name.strip_prefix(NS_PREFIX)?;
        for ext in self.extensions.iter() {
            let pfx = format!("{}_", ext.name());
            if let Some(bare) = rest.strip_prefix(&pfx) {
                return Some((ext.as_ref(), bare));
            }
        }
        None
    }

    /// Whether a bare tool is a write (author) tool for `ext`.
    fn is_write(ext: &dyn Extension, bare: &str) -> bool {
        ext.mcp_tools().iter().any(|t| t.name == bare && t.write)
    }

    /// Dispatch one namespaced extension tool call against `handle`'s canvas.
    ///
    /// Read: load the model from the root blob and project it.
    /// Author: load the model, apply the edit, persist the NEW model + the reconciled
    /// object diff through the SAME `apply_op` funnel, and reply with the new model.
    pub async fn dispatch(
        &self,
        handle: &ActorHandle,
        tool_name: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let (ext, bare) =
            self.split(tool_name).ok_or_else(|| format!("no such extension tool: {tool_name}"))?;
        let scene = handle.get_scene().await;
        let model = load_model(&scene, ext.name()).unwrap_or_else(|| ext.empty_model());

        if !Self::is_write(ext, bare) {
            return ext.read(&model, bare, args);
        }

        let new_model = ext.author(&model, bare, args)?;
        let mut alloc = IdOrderAlloc::new(ext.name());
        let desired = ext.export(&new_model, &mut alloc)?;
        let ops = reconcile(&scene, ext.name(), desired)?;
        for op in ops {
            // The SAME funnel the core write tools use — one op-apply path. MCP
            // writes are attributed to actor "mcp" (no auth).
            // TODO(auth): real authn/authz attaches at the transport boundary.
            if let ApplyResult::Rejected { errors } = handle.apply_op(op, "mcp").await {
                return Err(errors.join("; "));
            }
        }
        Ok(new_model)
    }
}

/// Load an extension's DomainModel JSON from its root object's `meta[extModel]`.
/// `None` when no root exists yet (fresh canvas) — the caller defaults to
/// `empty_model()`.
fn load_model(scene: &ObjectScene, ext_name: &str) -> Option<Value> {
    scene.objects.iter().find_map(|o| {
        let meta = o.meta.as_ref()?;
        if meta.get(META_EXT_KEY)?.as_str()? != ext_name {
            return None;
        }
        if meta.get(META_DOMAIN_KEY)?.as_str()? != "root" {
            return None;
        }
        meta.get(META_MODEL_KEY).cloned()
    })
}

/// The objects currently in `scene` that belong to `ext_name`, keyed by id.
fn current_ext_objects<'a>(
    scene: &'a ObjectScene,
    ext_name: &str,
) -> BTreeMap<String, &'a Object> {
    scene
        .objects
        .iter()
        .filter(|o| {
            o.meta
                .as_ref()
                .and_then(|m| m.get(META_EXT_KEY))
                .and_then(Value::as_str)
                == Some(ext_name)
        })
        .map(|o| (o.id.clone(), o))
        .collect()
}

/// Diff the desired export (a list of `InsertObject` ops) against the current
/// scene objects for `ext_name`, producing the MINIMAL op set:
/// - a desired id absent from the scene => the original `InsertObject`,
/// - a desired id present but differing => targeted property ops (no re-insert, no
///   `EditGeometry` — geometry keys are stable),
/// - a current id absent from the desired set => `Delete`.
/// This keeps re-export idempotent and transform-only (zero-rebake).
fn reconcile(
    scene: &ObjectScene,
    ext_name: &str,
    desired_ops: Vec<ObjectOp>,
) -> Result<Vec<ObjectOp>, String> {
    let current = current_ext_objects(scene, ext_name);

    // Index the desired objects (every export op is an InsertObject), preserving
    // order so inserts stay parents-first.
    let mut desired_order: Vec<String> = Vec::with_capacity(desired_ops.len());
    let mut desired: BTreeMap<String, Object> = BTreeMap::new();
    for op in desired_ops {
        match op {
            ObjectOp::InsertObject { object } => {
                desired_order.push(object.id.clone());
                desired.insert(object.id.clone(), object);
            }
            other => {
                return Err(format!(
                    "extension export must emit only insert-object ops, got {}",
                    other.kind()
                ))
            }
        }
    }

    let mut ops: Vec<ObjectOp> = Vec::new();

    // Inserts + updates, in desired (parents-first) order.
    for id in &desired_order {
        let want = &desired[id];
        match current.get(id) {
            None => ops.push(ObjectOp::InsertObject { object: want.clone() }),
            Some(have) if *have == want => {} // unchanged: emit nothing.
            Some(have) if carries_model_blob(want) => {
                // The CHILDLESS root holds the model blob in `meta[extModel]`, which
                // no property op can update. It has no children, so refresh it with a
                // cheap delete+reinsert of just this one tiny object — no cascade, no
                // rebake of real content. Skip when byte-identical (handled above).
                let _ = have;
                ops.push(ObjectOp::Delete { id: id.clone() });
                ops.push(ObjectOp::InsertObject { object: want.clone() });
            }
            Some(have) => ops.extend(field_diff(have, want)),
        }
    }

    // Deletions: a current ext object no longer desired.
    for id in current.keys() {
        if !desired.contains_key(id) {
            ops.push(ObjectOp::Delete { id: id.clone() });
        }
    }

    Ok(ops)
}

/// True when an object carries the model blob (`meta[extModel]`) — i.e. it is the
/// per-extension root the host refreshes via delete+reinsert.
fn carries_model_blob(object: &Object) -> bool {
    object
        .meta
        .as_ref()
        .is_some_and(|m| m.contains_key(META_MODEL_KEY))
}

/// Targeted per-field diff for an existing object: emit only the property ops whose
/// value changed. Covers exactly the fields the export seams mutate on re-export
/// (transform, text, style, meta name/flags, anchors, layout, sizing, reparent,
/// z-order); geometry is intentionally excluded — stable keys keep it byte-identical,
/// so a re-export is transform-only (zero-rebake). Equal objects emit nothing.
fn field_diff(have: &Object, want: &Object) -> Vec<ObjectOp> {
    let mut ops = Vec::new();
    let id = want.id.clone();

    if have.transform != want.transform {
        ops.push(ObjectOp::SetTransform { id: id.clone(), transform: want.transform });
    }
    if have.text != want.text {
        ops.push(ObjectOp::SetText { id: id.clone(), text: want.text.clone() });
    }
    if have.fill != want.fill || have.stroke != want.stroke {
        ops.push(ObjectOp::SetStyle {
            id: id.clone(),
            fill: Some(FieldEdit::from_option(want.fill.clone())),
            stroke: Some(FieldEdit::from_option(want.stroke.clone())),
        });
    }
    if have.anchors != want.anchors {
        ops.push(ObjectOp::SetAnchor { id: id.clone(), anchors: want.anchors.clone() });
    }
    if have.layout != want.layout {
        ops.push(ObjectOp::SetLayout { id: id.clone(), layout: want.layout });
    }
    if have.sizing != want.sizing {
        ops.push(ObjectOp::SetSizing { id: id.clone(), sizing: want.sizing });
    }
    if have.parent != want.parent {
        ops.push(ObjectOp::Reparent {
            id: id.clone(),
            parent: want.parent.clone(),
            order: want.order.clone(),
        });
    } else if have.order != want.order {
        // Reparent already carries `order`; only same-parent z-order drift needs a
        // standalone Reorder. Export re-derives `order` from emission sequence, so an
        // insert/removal/reorder that shifts a sibling's slot lands here — without
        // this the stored z-order would silently diverge from the intended export.
        ops.push(ObjectOp::Reorder { id: id.clone(), order: want.order.clone() });
    }
    // Panel meta (name/hidden/locked) rides `SetMeta`. The `meta[extModel]` blob is
    // NOT touched here — only the childless root carries it, and the host refreshes
    // that via delete+reinsert (see `reconcile`), so non-root objects never differ
    // on the blob.
    let name_changed = have.name != want.name;
    let hidden_changed = have.hidden != want.hidden;
    let locked_changed = have.locked != want.locked;
    if name_changed || hidden_changed || locked_changed {
        ops.push(ObjectOp::SetMeta {
            id: id.clone(),
            name: if name_changed {
                Some(FieldEdit::from_option(want.name.clone()))
            } else {
                None
            },
            hidden: if hidden_changed { Some(want.hidden) } else { None },
            locked: if locked_changed { Some(want.locked) } else { None },
        });
    }

    ops
}

/// A JSON projection of every advertised extension tool, for an HTTP catalog route
/// mirroring `/api/templates`. (Read-only; no shared state.)
pub fn extension_tools_json(registry: &ExtensionRegistry) -> Value {
    let tools: Vec<Value> = registry
        .tool_defs()
        .into_iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "schema": t.schema }))
        .collect();
    json!({ "tools": tools })
}

#[cfg(test)]
mod tests {
    //! Falsifiable guards for the incremental-reconcile seam — the host path that
    //! turns `export()` InsertObject ops into a MINIMAL op set and funnels them
    //! through the one core apply. Each test drives the REAL extensions (kanban /
    //! diagram) and the REAL `apply_object_op` (no second apply): author -> export ->
    //! `reconcile` against the live scene -> apply the result back -> assert the
    //! claimed invariant. They fail if reconcile degrades to delete-and-reinsert,
    //! drops a `field_diff` branch, or silently diverges z-order.
    use super::*;
    use shape_ext_todo_kanban::KanbanExtension;
    use shape_scene_core::object::apply_object_op;

    /// Apply a list of ops through the REAL core onto `scene` (no second apply).
    fn apply_all(scene: &mut ObjectScene, ops: Vec<ObjectOp>) {
        for op in ops {
            apply_object_op(scene, op).expect("op applies through the real core");
        }
    }

    /// Author -> export -> reconcile against `scene` -> apply the diff back, returning
    /// the reconcile op set (the thing under test). Mirrors `dispatch`'s write path.
    fn author_reconcile_apply(
        ext: &dyn Extension,
        scene: &mut ObjectScene,
        tool: &str,
        args: &Value,
    ) -> Vec<ObjectOp> {
        let model = load_model(scene, ext.name()).unwrap_or_else(|| ext.empty_model());
        let new_model = ext.author(&model, tool, args).expect("author");
        let mut alloc = IdOrderAlloc::new(ext.name());
        let desired = ext.export(&new_model, &mut alloc).expect("export");
        let ops = reconcile(scene, ext.name(), desired).expect("reconcile");
        apply_all(scene, ops.clone());
        ops
    }

    fn op_kinds(ops: &[ObjectOp]) -> Vec<&'static str> {
        ops.iter().map(ObjectOp::kind).collect()
    }

    /// A fresh board on an empty scene: the FIRST reconcile is all InsertObject (the
    /// insert branch), since nothing pre-exists.
    #[test]
    fn first_export_over_empty_scene_is_all_inserts() {
        let ext = KanbanExtension;
        let mut scene = ObjectScene::default();
        let ops = author_reconcile_apply(
            &ext,
            &mut scene,
            "add_column",
            &json!({ "id": "todo", "title": "To do" }),
        );
        assert!(
            ops.iter().all(|o| matches!(o, ObjectOp::InsertObject { .. })),
            "first export inserts only: {:?}",
            op_kinds(&ops)
        );
        // Root + board row + the one column = 3 objects on the scene.
        assert_eq!(scene.objects.len(), 3);
    }

    /// Re-exporting the SAME model emits NOTHING (every object byte-identical). This
    /// is the minimal-op claim: it FAILS if reconcile reverts to delete-and-reinsert.
    #[test]
    fn reexport_of_unchanged_model_emits_no_ops() {
        let ext = KanbanExtension;
        let mut scene = ObjectScene::default();
        author_reconcile_apply(&ext, &mut scene, "add_column", &json!({ "id": "todo" }));
        author_reconcile_apply(
            &ext,
            &mut scene,
            "add_card",
            &json!({ "id": "k1", "column": "todo", "title": "First" }),
        );

        // Now reconcile the SAME (unchanged) model against the populated scene.
        let model = load_model(&scene, ext.name()).unwrap();
        let mut alloc = IdOrderAlloc::new(ext.name());
        let desired = ext.export(&model, &mut alloc).unwrap();
        let ops = reconcile(&scene, ext.name(), desired).unwrap();
        assert!(ops.is_empty(), "unchanged re-export is a no-op, got {:?}", op_kinds(&ops));
    }

    /// `set_done` re-export is TARGETED: the card object id is REUSED (no Delete +
    /// no re-Insert of the card), and its style flips to the done preset via a single
    /// SetStyle. Guards both the field_diff style branch and the set_done EFFECT.
    #[test]
    fn set_done_reexports_as_a_targeted_style_op_with_a_reused_id() {
        let ext = KanbanExtension;
        let mut scene = ObjectScene::default();
        author_reconcile_apply(&ext, &mut scene, "add_column", &json!({ "id": "todo" }));
        author_reconcile_apply(
            &ext,
            &mut scene,
            "add_card",
            &json!({ "id": "k1", "column": "todo", "title": "First" }),
        );

        let card_id = IdOrderAlloc::new(ext.name()).id("card:k1");
        let fill_before = scene.get(&card_id).unwrap().fill.clone();

        let ops = author_reconcile_apply(&ext, &mut scene, "set_done", &json!({ "card": "k1", "done": true }));

        // The card is updated by exactly ONE targeted SetStyle — never deleted and
        // reinserted. (The childless root is refreshed separately because its model
        // blob changed; that is the only delete+insert, asserted elsewhere.)
        let card_style_ops = ops
            .iter()
            .filter(|o| matches!(o, ObjectOp::SetStyle { id, .. } if *id == card_id))
            .count();
        assert_eq!(card_style_ops, 1, "card flips via one SetStyle: {:?}", op_kinds(&ops));
        assert!(
            !ops.iter().any(|o| matches!(o, ObjectOp::Delete { id } if *id == card_id)),
            "the card is never delete-and-reinserted"
        );
        // The card object is the SAME id and its fill actually changed (the effect).
        let card = scene.get(&card_id).expect("card id reused, not reinserted");
        let (done_fill, _, _) =
            shape_scene_core::object::semantic_preset_style("artifact");
        assert_eq!(card.fill, done_fill, "done flips the card to the artifact preset");
        assert_ne!(card.fill, fill_before, "the style actually changed from the open preset");
    }

    /// Editing the model refreshes ONLY the childless root via delete+reinsert (it
    /// carries `meta[extModel]`, which no property op can update); every other object
    /// is touched by a targeted op or untouched — never the root's children.
    #[test]
    fn model_edit_refreshes_only_the_root_via_delete_reinsert() {
        let ext = KanbanExtension;
        let mut scene = ObjectScene::default();
        author_reconcile_apply(&ext, &mut scene, "add_column", &json!({ "id": "todo" }));
        author_reconcile_apply(
            &ext,
            &mut scene,
            "add_card",
            &json!({ "id": "k1", "column": "todo", "title": "First" }),
        );

        let root_id = IdOrderAlloc::new(ext.name()).id("root");
        let card_id = IdOrderAlloc::new(ext.name()).id("card:k1");

        let model = load_model(&scene, ext.name()).unwrap();
        let new_model = ext.author(&model, "set_done", &json!({ "card": "k1", "done": true })).unwrap();
        let mut alloc = IdOrderAlloc::new(ext.name());
        let desired = ext.export(&new_model, &mut alloc).unwrap();
        let ops = reconcile(&scene, ext.name(), desired).unwrap();

        // The root is the only id that is deleted-and-reinserted (its blob changed).
        let deleted: Vec<&str> = ops
            .iter()
            .filter_map(|o| match o {
                ObjectOp::Delete { id } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deleted, vec![root_id.as_str()], "only the root is delete+reinserted");
        // The card is updated in place (SetStyle), not deleted.
        assert!(
            ops.iter().any(|o| matches!(o, ObjectOp::SetStyle { id, .. } if *id == card_id)),
            "card style updated in place, not via reinsert"
        );
    }

    /// THE finding-3/4 regression guard: an edit that shifts a sibling's emission
    /// slot (here, moving the MIDDLE card out of a column) MUST re-key the shifted
    /// sibling's z-order. After reconcile+apply, every surviving object's stored
    /// `order` equals the freshly-exported order — it FAILS if field_diff drops the
    /// order field (the append-only-only framework hack).
    #[test]
    fn mid_sequence_move_reorders_shifted_siblings() {
        let ext = KanbanExtension;
        let mut scene = ObjectScene::default();
        author_reconcile_apply(&ext, &mut scene, "add_column", &json!({ "id": "todo" }));
        author_reconcile_apply(&ext, &mut scene, "add_column", &json!({ "id": "done" }));
        for c in ["k1", "k2", "k3"] {
            author_reconcile_apply(
                &ext,
                &mut scene,
                "add_card",
                &json!({ "id": c, "column": "todo", "title": c }),
            );
        }

        // Move the MIDDLE card (k2) to the other column. k3 stays under "todo" but
        // shifts from emission slot 3 to slot 2 — its order must be re-keyed.
        let ops = author_reconcile_apply(
            &ext,
            &mut scene,
            "move_card",
            &json!({ "card": "k2", "toColumn": "done" }),
        );
        assert!(
            ops.iter().any(|o| matches!(o, ObjectOp::Reorder { .. }) || matches!(o, ObjectOp::Reparent { .. })),
            "a shifted sibling re-keys its z-order: {:?}",
            op_kinds(&ops)
        );

        // The invariant: after applying the diff, every surviving object's stored
        // order equals the freshly-exported order (no stale, silently-dropped order).
        let model = load_model(&scene, ext.name()).unwrap();
        let mut alloc = IdOrderAlloc::new(ext.name());
        let fresh = ext.export(&model, &mut alloc).unwrap();
        for op in &fresh {
            let ObjectOp::InsertObject { object } = op else { continue };
            let stored = scene.get(&object.id).expect("object present after reconcile");
            assert_eq!(
                stored.order, object.order,
                "stored z-order of {} matches the fresh export (no order drift)",
                object.id
            );
        }
    }

    /// A diagram `connect` over an existing 2-node scene emits ONLY the new edge's
    /// InsertObject — the pre-existing node objects stay untouched (no churn).
    #[test]
    fn diagram_connect_inserts_only_the_new_edge() {
        let ext = shape_ext_structure_diagram::DiagramExtension;
        let mut scene = ObjectScene::default();
        author_reconcile_apply(&ext, &mut scene, "add_node", &json!({ "id": "a", "x": 0.0, "y": 0.0 }));
        author_reconcile_apply(&ext, &mut scene, "add_node", &json!({ "id": "b", "x": 300.0, "y": 0.0 }));

        let ops = author_reconcile_apply(
            &ext,
            &mut scene,
            "connect",
            &json!({ "id": "e1", "from": "a", "to": "b" }),
        );
        // The model blob changed, so the root is refreshed; the genuinely new object
        // is the edge connector. No node is deleted/reinserted.
        let edge_id = IdOrderAlloc::new(ext.name()).id("edge:e1");
        let root_id = IdOrderAlloc::new(ext.name()).id("root");
        let inserted: Vec<&str> = ops
            .iter()
            .filter_map(|o| match o {
                ObjectOp::InsertObject { object } => Some(object.id.as_str()),
                _ => None,
            })
            .collect();
        let mut inserted_sorted = inserted.clone();
        inserted_sorted.sort_unstable();
        let mut want = vec![edge_id.as_str(), root_id.as_str()];
        want.sort_unstable();
        assert_eq!(inserted_sorted, want, "only the new edge + the refreshed root insert");

        // The relational invariant THROUGH the incremental path: after the edge
        // lands via reconcile (not a first-shot full export), the core graph reads
        // it with BOTH endpoints (a->b), never a single-target collapse. Deterministic
        // because the root delete+reinsert touches no anchored object.
        let node_a = IdOrderAlloc::new(ext.name()).id("node:a");
        let node_b = IdOrderAlloc::new(ext.name()).id("node:b");
        assert_eq!(
            shape_scene_core::object::connection_graph(&scene),
            vec![(node_a, node_b)],
            "the incrementally-applied edge carries both endpoints (a->b)"
        );
    }
}
