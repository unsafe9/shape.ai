//! Insert-primitive helper (CC0.3) — a pure map from a toolbar primitive kind +
//! an anchor world point to the [`RenderScenePatch`] op(s) that materialize it.
//!
//! This is the scene-core port of `App.svelte`'s `insertPrimitive` /
//! `buildPrimitiveOps` / `primitiveCardSpec`: the cockpit shell arms an
//! [`crate::tool::ActiveTool`] insert tool, then asks scene-core for the ops to
//! apply. Keeping the mapping here means every shell inserts identical objects
//! (same card types, dims, copy, and connector geometry) without re-declaring
//! the table.
//!
//! The mapping mirrors the TS exactly:
//! * `rectangle` -> one `create-card` type `task`, 220x140, title "Rectangle".
//! * `ellipse`   -> one `create-card` type `option`, 200x200, title "Ellipse".
//! * `sticky`    -> one `create-card` type `proposition`, 220x180, title "Note",
//!   summary "Type your note here.".
//! * `frame`     -> one `create-group`, 640x440 centered on the anchor.
//! * `connector` -> two small `task` anchor cards (28x28) joined by a
//!   `create-edge` labelled "connects".
//!
//! Purity: all identity (`id`) and time (`now`) are injected, never sourced from
//! the ambient environment. A single-card primitive needs one id; the connector
//! needs three (source card, target card, edge), supplied via [`InsertIds`].

use serde::{Deserialize, Serialize};

use crate::model::{WorldPoint, WorldRect};
use crate::op::{RenderCard, RenderGroup, RenderScenePatch};

/// The toolbar primitive kinds the insert tools can place. Mirrors the TS
/// `PrimitiveKindId` union (`rectangle | ellipse | connector | sticky | frame`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveSpec {
    Rectangle,
    Ellipse,
    Connector,
    Sticky,
    Frame,
}

impl PrimitiveSpec {
    /// Parse a primitive kind token (`"rectangle"`, `"ellipse"`, `"connector"`,
    /// `"sticky"`, `"frame"`). Returns `None` for an unknown token.
    pub fn from_token(token: &str) -> Option<PrimitiveSpec> {
        match token {
            "rectangle" => Some(PrimitiveSpec::Rectangle),
            "ellipse" => Some(PrimitiveSpec::Ellipse),
            "connector" => Some(PrimitiveSpec::Connector),
            "sticky" => Some(PrimitiveSpec::Sticky),
            "frame" => Some(PrimitiveSpec::Frame),
            _ => None,
        }
    }
}

/// Injected ids for an insert. A single-card primitive uses `primary`; the
/// connector additionally uses `secondary` (the target anchor card) and `edge`.
/// Frame uses `primary` as the group id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertIds {
    /// The id of the created card / group (or the connector's source card).
    pub primary: String,
    /// The connector's target anchor-card id. Ignored by non-connector kinds.
    pub secondary: String,
    /// The connector's edge id. Ignored by non-connector kinds.
    pub edge: String,
}

/// Build the render op(s) that insert `spec` at `anchor` inside `group_id`.
///
/// `anchor` is the world point the primitive is centered on (e.g. the viewport
/// center). `group_id` is the host frame the card/edge is created in; it is
/// ignored for `Frame` (a frame *is* a group, so it has no host). `ids` supplies
/// the injected identity. `_now` is accepted for signature parity with the rest
/// of scene-core's injected-time seam; the ops themselves carry no timestamp
/// (apply stamps `updated_at`).
///
/// Returns the ops in apply order — for the connector, the two anchor cards
/// precede the edge so the edge validates against live node ids when run as a
/// `batch`.
pub fn insert_primitive_ops(
    spec: PrimitiveSpec,
    anchor: WorldPoint,
    group_id: &str,
    ids: &InsertIds,
    _now: &str,
) -> Vec<RenderScenePatch> {
    match spec {
        PrimitiveSpec::Frame => vec![RenderScenePatch::CreateGroup {
            group: RenderGroup {
                id: ids.primary.clone(),
                title: "Frame".to_string(),
                summary: String::new(),
                bounds: WorldRect {
                    x: anchor.x - 320.0,
                    y: anchor.y - 220.0,
                    width: 640.0,
                    height: 440.0,
                },
                tag_ids: vec![],
                z_index: 0.0,
                style_key: "default".to_string(),
            },
        }],
        PrimitiveSpec::Connector => {
            let handle_w = 28.0;
            let handle_h = 28.0;
            let start_x = anchor.x - 150.0;
            let start_y = anchor.y;
            let end_x = start_x + 320.0;
            let end_y = start_y;
            vec![
                RenderScenePatch::CreateCard {
                    card: primitive_card(
                        &ids.primary,
                        group_id,
                        "task",
                        WorldRect {
                            x: start_x,
                            y: start_y,
                            width: handle_w,
                            height: handle_h,
                        },
                        0.0,
                        "Line start",
                        "",
                    ),
                },
                RenderScenePatch::CreateCard {
                    card: primitive_card(
                        &ids.secondary,
                        group_id,
                        "task",
                        WorldRect {
                            x: end_x,
                            y: end_y,
                            width: handle_w,
                            height: handle_h,
                        },
                        1.0,
                        "Line end",
                        "",
                    ),
                },
                RenderScenePatch::CreateEdge {
                    group_id: group_id.to_string(),
                    source: ids.primary.clone(),
                    target: ids.secondary.clone(),
                    edge_id: ids.edge.clone(),
                    label: Some("connects".to_string()),
                },
            ]
        }
        // Card primitives: one create-card centered on the anchor.
        _ => {
            let (node_type, width, height, title, summary) = card_spec(spec);
            let bounds = WorldRect {
                x: anchor.x - width / 2.0,
                y: anchor.y - height / 2.0,
                width,
                height,
            };
            vec![RenderScenePatch::CreateCard {
                card: primitive_card(&ids.primary, group_id, node_type, bounds, 0.0, title, summary),
            }]
        }
    }
}

/// The card type / size / copy for a single-card primitive, mirroring the TS
/// `primitiveCardSpec`. Only called for `Rectangle`/`Ellipse`/`Sticky`.
fn card_spec(spec: PrimitiveSpec) -> (&'static str, f64, f64, &'static str, &'static str) {
    match spec {
        PrimitiveSpec::Rectangle => ("task", 220.0, 140.0, "Rectangle", ""),
        PrimitiveSpec::Ellipse => ("option", 200.0, 200.0, "Ellipse", ""),
        // sticky / text box: a card whose body is editable text.
        PrimitiveSpec::Sticky => ("proposition", 220.0, 180.0, "Note", "Type your note here."),
        // Frame/Connector never reach here.
        PrimitiveSpec::Frame | PrimitiveSpec::Connector => ("task", 220.0, 140.0, "", ""),
    }
}

/// Build a [`RenderCard`] the way the TS `primitiveCard` does: `status` "draft",
/// `style_key == type`, accessibility label `"{type} {title}"`.
fn primitive_card(
    id: &str,
    group_id: &str,
    node_type: &str,
    bounds: WorldRect,
    z_index: f64,
    title: &str,
    summary: &str,
) -> RenderCard {
    RenderCard {
        id: id.to_string(),
        group_id: group_id.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        detail: String::new(),
        status: "draft".to_string(),
        node_type: node_type.to_string(),
        bounds,
        z_index,
        style_key: node_type.to_string(),
        accessibility_label: format!("{node_type} {title}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::apply_render_patch_to_shape_scene;
    use crate::model::{Scene, SceneSelection};

    const NOW: &str = "2026-06-07T00:00:00.000Z";

    fn ids() -> InsertIds {
        InsertIds {
            primary: "p-1".to_string(),
            secondary: "p-2".to_string(),
            edge: "e-1".to_string(),
        }
    }

    fn anchor() -> WorldPoint {
        WorldPoint { x: 100.0, y: 200.0 }
    }

    #[test]
    fn from_token_round_trip() {
        assert_eq!(
            PrimitiveSpec::from_token("rectangle"),
            Some(PrimitiveSpec::Rectangle)
        );
        assert_eq!(
            PrimitiveSpec::from_token("ellipse"),
            Some(PrimitiveSpec::Ellipse)
        );
        assert_eq!(
            PrimitiveSpec::from_token("connector"),
            Some(PrimitiveSpec::Connector)
        );
        assert_eq!(
            PrimitiveSpec::from_token("sticky"),
            Some(PrimitiveSpec::Sticky)
        );
        assert_eq!(PrimitiveSpec::from_token("frame"), Some(PrimitiveSpec::Frame));
        assert_eq!(PrimitiveSpec::from_token("bogus"), None);
    }

    #[test]
    fn rectangle_maps_to_task_card_220x140_centered() {
        let ops = insert_primitive_ops(PrimitiveSpec::Rectangle, anchor(), "g-1", &ids(), NOW);
        assert_eq!(ops.len(), 1);
        let RenderScenePatch::CreateCard { card } = &ops[0] else {
            panic!("expected create-card, got {:?}", ops[0]);
        };
        assert_eq!(card.node_type, "task");
        assert_eq!(card.style_key, "task");
        assert_eq!(card.title, "Rectangle");
        assert_eq!(card.group_id, "g-1");
        assert_eq!(card.bounds.width, 220.0);
        assert_eq!(card.bounds.height, 140.0);
        // Centered on the anchor.
        assert_eq!(card.bounds.x, 100.0 - 110.0);
        assert_eq!(card.bounds.y, 200.0 - 70.0);
    }

    #[test]
    fn ellipse_maps_to_option_card_200x200() {
        let ops = insert_primitive_ops(PrimitiveSpec::Ellipse, anchor(), "g-1", &ids(), NOW);
        let RenderScenePatch::CreateCard { card } = &ops[0] else {
            panic!("expected create-card");
        };
        assert_eq!(card.node_type, "option");
        assert_eq!(card.bounds.width, 200.0);
        assert_eq!(card.bounds.height, 200.0);
        assert_eq!(card.title, "Ellipse");
    }

    #[test]
    fn sticky_maps_to_proposition_card_220x180_with_copy() {
        let ops = insert_primitive_ops(PrimitiveSpec::Sticky, anchor(), "g-1", &ids(), NOW);
        let RenderScenePatch::CreateCard { card } = &ops[0] else {
            panic!("expected create-card");
        };
        assert_eq!(card.node_type, "proposition");
        assert_eq!(card.bounds.width, 220.0);
        assert_eq!(card.bounds.height, 180.0);
        assert_eq!(card.title, "Note");
        assert_eq!(card.summary, "Type your note here.");
    }

    #[test]
    fn frame_maps_to_create_group_640x440_centered() {
        let ops = insert_primitive_ops(PrimitiveSpec::Frame, anchor(), "ignored", &ids(), NOW);
        assert_eq!(ops.len(), 1);
        let RenderScenePatch::CreateGroup { group } = &ops[0] else {
            panic!("expected create-group, got {:?}", ops[0]);
        };
        assert_eq!(group.id, "p-1");
        assert_eq!(group.title, "Frame");
        assert_eq!(group.bounds.width, 640.0);
        assert_eq!(group.bounds.height, 440.0);
        assert_eq!(group.bounds.x, 100.0 - 320.0);
        assert_eq!(group.bounds.y, 200.0 - 220.0);
    }

    #[test]
    fn connector_maps_to_two_cards_plus_edge() {
        let ops = insert_primitive_ops(PrimitiveSpec::Connector, anchor(), "g-1", &ids(), NOW);
        assert_eq!(ops.len(), 3);
        // Two anchor cards first, edge last (so it validates against live ids).
        let RenderScenePatch::CreateCard { card: src } = &ops[0] else {
            panic!("expected source create-card");
        };
        let RenderScenePatch::CreateCard { card: dst } = &ops[1] else {
            panic!("expected target create-card");
        };
        let RenderScenePatch::CreateEdge {
            group_id,
            source,
            target,
            edge_id,
            label,
        } = &ops[2]
        else {
            panic!("expected create-edge");
        };
        assert_eq!(src.id, "p-1");
        assert_eq!(dst.id, "p-2");
        assert_eq!(src.bounds.width, 28.0);
        assert_eq!(dst.bounds.width, 28.0);
        // Geometry mirrors TS: start at anchor.x-150, end +320, same y.
        assert_eq!(src.bounds.x, 100.0 - 150.0);
        assert_eq!(dst.bounds.x, (100.0 - 150.0) + 320.0);
        assert_eq!(src.bounds.y, 200.0);
        assert_eq!(dst.bounds.y, 200.0);
        assert_eq!(group_id, "g-1");
        assert_eq!(source, "p-1");
        assert_eq!(target, "p-2");
        assert_eq!(edge_id, "e-1");
        assert_eq!(label.as_deref(), Some("connects"));
    }

    #[test]
    fn ops_apply_cleanly_against_a_live_group() {
        // Build a scene with one host group, then apply each primitive's ops.
        let base = {
            let scene = Scene {
                version: 1,
                scene_version: 0,
                groups: vec![],
                nodes: vec![],
                edges: vec![],
                tags: vec![],
                comments: vec![],
                artifacts: vec![],
                proposals: None,
                selection: SceneSelection::Canvas,
                updated_at: NOW.to_string(),
            };
            // create the host frame first
            let frame_ops =
                insert_primitive_ops(PrimitiveSpec::Frame, anchor(), "ignored", &ids(), NOW);
            let mut s = scene;
            for op in frame_ops {
                let r = apply_render_patch_to_shape_scene(&s, &op, NOW, None);
                assert!(r.errors.is_empty(), "frame apply errors: {:?}", r.errors);
                s = r.scene;
            }
            s
        };
        let host_id = base.groups[0].id.clone();

        // A card primitive applies and lands in the host group.
        let card_ids = InsertIds {
            primary: "rect-1".to_string(),
            secondary: String::new(),
            edge: String::new(),
        };
        let rect_ops =
            insert_primitive_ops(PrimitiveSpec::Rectangle, anchor(), &host_id, &card_ids, NOW);
        let mut s = base.clone();
        for op in rect_ops {
            let r = apply_render_patch_to_shape_scene(&s, &op, NOW, None);
            assert!(r.errors.is_empty(), "rect apply errors: {:?}", r.errors);
            s = r.scene;
        }
        assert_eq!(s.nodes.len(), 1);
        assert_eq!(s.nodes[0].group_id, host_id);

        // The connector's three ops apply cleanly (two cards then the edge).
        let conn_ids = InsertIds {
            primary: "c-src".to_string(),
            secondary: "c-dst".to_string(),
            edge: "c-edge".to_string(),
        };
        let conn_ops =
            insert_primitive_ops(PrimitiveSpec::Connector, anchor(), &host_id, &conn_ids, NOW);
        let mut s2 = base;
        for op in conn_ops {
            let r = apply_render_patch_to_shape_scene(&s2, &op, NOW, None);
            assert!(r.errors.is_empty(), "connector apply errors: {:?}", r.errors);
            s2 = r.scene;
        }
        assert_eq!(s2.nodes.len(), 2);
        assert_eq!(s2.edges.len(), 1);
        assert_eq!(s2.edges[0].source, "c-src");
        assert_eq!(s2.edges[0].target, "c-dst");
    }
}
