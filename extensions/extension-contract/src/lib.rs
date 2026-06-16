//! The data-representation extension contract.
//!
//! An extension is a self-contained plugin that RIDES ON the one core canvas: it
//! owns a typed domain model (NOT scene-core types) and exposes the seams the host
//! calls. There is NO platform-owned per-domain core — the platform owns only the
//! scene types and the MCP infra.
//!
//! The seams (each the minimal shape the two reference extensions need, matching
//! the proven `templates::build_template` pattern):
//!
//! 1. **mcp-register** — `mcp_tools() -> Vec<McpToolMeta>` (self-describing, like
//!    `object_mcp_tools`) + `read(model, tool, args) -> Value` and
//!    `author(model, tool, args) -> Result<Vec<DomainEdit>, String>`. Domain edits
//!    mutate the DOMAIN MODEL only, never the scene directly. This is the AI
//!    authoring surface for the domain vocabulary (board/column/card,
//!    node/edge), not the low-level object vocabulary.
//!
//! 2. **export-to-scene** — `export(model, alloc) -> Vec<ObjectOp>` lowering the
//!    domain model to scene-core objects, emitting ONLY [`ObjectOp`]s so the
//!    single op-apply path stays authoritative (no second apply anywhere). Each
//!    emitted object carries `meta["ext"] = <name>` + a stable domain key, so an
//!    incremental re-export reuses ids (transform-only / zero-rebake). Pure:
//!    ids/order come from the injected [`IdOrderAlloc`], exactly like
//!    `templates::build_template`.
//!
//! Authoring flows one way: MCP author -> `DomainEdit` -> `DomainModel` ->
//! `export()` -> `ObjectOp`s -> the host's `apply_object_op` funnel.

use serde::Serialize;
use serde_json::Value;
use shape_scene_core::object::ObjectOp;

pub use shape_scene_core::object::ObjectMeta;

/// Reserved `Object.meta` key carrying the owning extension's name on every
/// exported object (and on the per-extension root object). Lets a round-trip and
/// an incremental re-export find an extension's objects.
pub const META_EXT_KEY: &str = "ext";

/// Reserved `Object.meta` key carrying the stable domain key of an exported object
/// (`col:<id>`, `card:<id>`, `node:<id>`, `edge:<id>`, `root`). The export seam
/// keys ids off this so re-export reuses ids instead of delete-and-reinsert.
pub const META_DOMAIN_KEY: &str = "extKey";

/// Reserved `Object.meta` key on the per-extension ROOT object carrying the
/// serialized [`DomainModel`] JSON. The model is the source of truth; persisting
/// it as a meta blob rides the existing op-apply/sync/storage path and keeps the
/// server stateless (no sidecar = no second source of truth).
pub const META_MODEL_KEY: &str = "extModel";

/// Self-describing metadata for one extension MCP tool. The same light shape
/// `object_mcp::McpToolMeta` uses, defined here so an extension crate depends on
/// scene-core + ui only (never the server). `name` is the bare domain tool name
/// (`add_card`); the host namespaces it `ext_<extname>_<name>` when advertising.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    /// Whether the tool mutates the model (`author`) or only reads it (`read`).
    pub write: bool,
    /// A light input schema, not a full JSON-Schema document.
    pub schema: Value,
}

/// Monotonic id + fractional-order allocator injected into the export seam, so
/// `export()` stays pure (no rng/time) and a fixed allocator yields byte-identical
/// ops. Mirrors the `id_alloc`/`order_alloc` closures of `templates::build_template`,
/// but as a struct because the contract holds it across trait calls.
///
/// `id(domain_key)` is STABLE: the same domain key always yields the same object
/// id, so re-exporting after a domain edit reuses ids (the incremental, zero-rebake
/// requirement). `order()` is sequential.
pub struct IdOrderAlloc {
    /// Object-id prefix (the extension name), so ids never collide across
    /// extensions: `ext-<name>-<domain-key>`.
    prefix: String,
    order_n: u64,
}

impl IdOrderAlloc {
    pub fn new(prefix: impl Into<String>) -> Self {
        IdOrderAlloc { prefix: prefix.into(), order_n: 0 }
    }

    /// The stable object id for a domain key. Deterministic and collision-free
    /// across extensions (prefixed by the extension name). Re-export with the same
    /// key returns the same id, so apply diffs to transform/property edits, not a
    /// delete-and-reinsert.
    pub fn id(&self, domain_key: &str) -> String {
        format!("ext-{}-{}", self.prefix, domain_key)
    }

    /// The next fractional-order key, in allocation sequence. Base-62-free simple
    /// monotone keys suffice for export (z-order within an extension subtree); they
    /// sort by plain `str` Ord, matching `Object.order`.
    pub fn order(&mut self) -> String {
        self.order_n += 1;
        format!("e{:08}", self.order_n)
    }
}

/// The one object-safe seam the host's extension registry dispatches over. An
/// in-tree extension is registered by boxing its handle into the registry at build
/// time (rmcp's `#[tool_router]` is compile-time, so this is build-time
/// composition, not a runtime plugin system — that is a later option).
///
/// The model is opaque to the host: each impl owns its typed `DomainModel` and
/// serializes it to/from the root object's `meta[extModel]` blob. The host only
/// ever sees `Value` (model JSON), `Vec<ObjectOp>` (export), and
/// `McpToolMeta`/`Value` (MCP) — so a new extension needs no host change beyond
/// one registration line.
pub trait Extension: Send + Sync {
    /// The extension name; the `ext_<name>_<tool>` MCP namespace and the `meta[ext]`
    /// tag.
    fn name(&self) -> &'static str;

    // --- seam 1: mcp-register ---

    /// The self-describing domain tool group. The host advertises each as
    /// `ext_<name>_<tool>` on the one `/mcp` endpoint.
    fn mcp_tools(&self) -> Vec<McpToolMeta>;

    /// A read tool: project the model to JSON. `tool` is the bare domain name.
    fn read(&self, model: &Value, tool: &str, args: &Value) -> Result<Value, String>;

    /// An author tool: apply the named domain edit to `model`, returning the NEW
    /// model JSON. The host then calls [`Extension::export`] over the new model and
    /// funnels the resulting ops through its single `apply_object_op` path. The
    /// model is mutated here; the scene is never touched directly.
    fn author(&self, model: &Value, tool: &str, args: &Value) -> Result<Value, String>;

    // --- seam 2: export-to-scene ---

    /// Lower the model to scene-core ops. Pure: ids/order from `alloc`. Emits only
    /// `ObjectOp`s; every emitted object carries `meta[ext]` + `meta[extKey]`, and
    /// the per-extension root object also carries `meta[extModel]` (the model
    /// blob), so the round-trip is closed and re-export reuses ids.
    fn export(&self, model: &Value, alloc: &mut IdOrderAlloc) -> Result<Vec<ObjectOp>, String>;

    /// The default (empty) domain model JSON for a fresh extension instance.
    fn empty_model(&self) -> Value;
}

/// Stamp the reserved tag keys (`ext`, `extKey`) onto an object's meta. The export
/// seam calls this on every emitted object so a round-trip / incremental re-export
/// can find the extension's objects. Convenience for impls — pure.
pub fn tag_meta(meta: &mut ObjectMeta, ext_name: &str, domain_key: &str) {
    meta.insert(META_EXT_KEY.to_string(), Value::String(ext_name.to_string()));
    meta.insert(META_DOMAIN_KEY.to_string(), Value::String(domain_key.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_alloc_is_stable_per_domain_key_and_prefixed() {
        let alloc = IdOrderAlloc::new("kanban");
        // Same domain key -> same id (the incremental-reuse contract).
        assert_eq!(alloc.id("col:todo"), alloc.id("col:todo"));
        assert_eq!(alloc.id("col:todo"), "ext-kanban-col:todo");
        // Different prefix never collides for the same key.
        let other = IdOrderAlloc::new("diagram");
        assert_ne!(alloc.id("node:1"), other.id("node:1"));
    }

    #[test]
    fn order_alloc_is_monotone_and_sorts_lexically() {
        let mut alloc = IdOrderAlloc::new("x");
        let a = alloc.order();
        let b = alloc.order();
        assert_ne!(a, b);
        assert!(a < b, "orders sort by plain str Ord: {a} < {b}");
    }

    #[test]
    fn tag_meta_stamps_both_reserved_keys() {
        let mut meta = ObjectMeta::new();
        tag_meta(&mut meta, "kanban", "card:1");
        assert_eq!(meta[META_EXT_KEY], Value::String("kanban".into()));
        assert_eq!(meta[META_DOMAIN_KEY], Value::String("card:1".into()));
    }
}
