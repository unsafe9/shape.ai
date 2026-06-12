//! Object-native persistence + apply layer: maps an [`ObjectScene`] onto the
//! store-neutral [`Record`] model so the spatial index can window large canvases.
//!
//! Record layout:
//! * Each [`Object`] is one `Record`, id `"{canvasId}:object:{objId}"`, kind
//!   [`KIND_OBJECT`]. Payload is the object's JSON (path-string `d`, not the
//!   parsed mirror — see [`Object`]'s serde contract). Placement-bearing objects
//!   carry a [`RegionKey`] (world-space AABB) for region queries.
//! * One canvas-meta `Record`, id `"{canvasId}:canvas"`, kind [`KIND_CANVAS`],
//!   holds scene-level fields (`sceneVersion`, `selection`, `tags`, `updatedAt`).
//!
//! Region indexing rides the storage core's [`SpatialStore`]. The store is built
//! over the sync [`StorageAdapter`] (the only surface `MemoryAdapter` implements,
//! so it stays testable) and over [`SpatialStore`] for indexed writes + queries.
//!
//! The store caches a per-canvas in-memory [`ObjectScene`] + [`PropertyStore`] so
//! an [`apply`](ObjectStore::apply) does not re-scan the backend per op;
//! [`load_scene`](ObjectStore::load_scene) rebuilds on a cold canvas. The actor
//! seam owns eviction, not this layer.

use std::collections::HashMap;

use shape_scene_core::object::{
    apply_object_op_lww, ApplyError, Geometry, Object, ObjectOp, ObjectScene, ObjectSelection,
    TagDef, GEOMETRY_QUANTUM_PER_PX,
};
use shape_scene_core::{CanvasId, PropertyStore};
use shape_storage_core::{Record, RegionKey, RegionWindow, SpatialStore, StorageAdapter};

pub const KIND_OBJECT: &str = "object";
pub const KIND_CANVAS: &str = "canvas";

#[derive(Debug)]
pub enum ObjectStoreError {
    /// Pure-core rejection of the op.
    Apply(ApplyError),
    Storage(shape_storage_core::StorageError),
}

impl core::fmt::Display for ObjectStoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ObjectStoreError::Apply(e) => write!(f, "apply error: {e}"),
            ObjectStoreError::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for ObjectStoreError {}

impl From<ApplyError> for ObjectStoreError {
    fn from(e: ApplyError) -> Self {
        ObjectStoreError::Apply(e)
    }
}

impl From<shape_storage_core::StorageError> for ObjectStoreError {
    fn from(e: shape_storage_core::StorageError) -> Self {
        ObjectStoreError::Storage(e)
    }
}

pub fn object_record_id(canvas_id: &CanvasId, object_id: &str) -> String {
    format!("{canvas_id}:{KIND_OBJECT}:{object_id}")
}

pub fn canvas_record_id(canvas_id: &CanvasId) -> String {
    format!("{canvas_id}:{KIND_CANVAS}")
}

/// `list()` is global id-sorted, so a prefix scan enumerates exactly one canvas's
/// object + canvas-meta records.
pub fn canvas_record_prefix(canvas_id: &CanvasId) -> String {
    format!("{canvas_id}:")
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanvasMeta {
    scene_version: i64,
    tags: Vec<TagDef>,
    selection: ObjectSelection,
    updated_at: String,
}

pub fn region_window_to_bbox(window: RegionWindow) -> (f64, f64, f64, f64) {
    (window.min_x, window.min_y, window.max_x, window.max_y)
}

/// World-space AABB [`RegionKey`], or `None` when the object has no geometry.
///
/// Approximate by design: hulls the four local-bbox corners mapped through the
/// object's [`Transform3x3`] without flattening curves. Bezier handles can bulge
/// slightly outside, which is fine for a query window — over-inclusion only widens
/// the candidate set; the exact refilter runs in the spatial store.
pub fn object_region_key(canvas_id: &CanvasId, object: &Object) -> Option<RegionKey> {
    let mut geometry = object.geometry.clone();
    // Hydrate from the path-string `d` (the at-rest/wire form); a parse failure
    // means no usable extents.
    if geometry.ensure_parsed().is_err() {
        return None;
    }
    let local = local_extents(&geometry)?;

    let per_px = f64::from(GEOMETRY_QUANTUM_PER_PX);
    let corners = [
        (local.0 as f64 / per_px, local.1 as f64 / per_px),
        (local.2 as f64 / per_px, local.1 as f64 / per_px),
        (local.2 as f64 / per_px, local.3 as f64 / per_px),
        (local.0 as f64 / per_px, local.3 as f64 / per_px),
    ];

    let t = &object.transform;
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (lx, ly) in corners {
        let (wx, wy) = t.apply_point(lx, ly);
        min_x = min_x.min(wx);
        min_y = min_y.min(wy);
        max_x = max_x.max(wx);
        max_y = max_y.max(wy);
    }

    Some(RegionKey {
        canvas_id: canvas_id.to_string(),
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

/// Object-local quantized i32 extents over all parsed node positions, or `None`
/// when there are no nodes.
fn local_extents(geometry: &Geometry) -> Option<(i32, i32, i32, i32)> {
    let mut seen = false;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for sp in &geometry.subpaths {
        for n in &sp.nodes {
            seen = true;
            min_x = min_x.min(n.x);
            min_y = min_y.min(n.y);
            max_x = max_x.max(n.x);
            max_y = max_y.max(n.y);
        }
    }
    seen.then_some((min_x, min_y, max_x, max_y))
}

/// `version` is stamped on the Record (the actor passes the server seq).
fn object_to_record(canvas_id: &CanvasId, object: &Object, version: u64) -> (Record, Option<RegionKey>) {
    let record = Record {
        id: object_record_id(canvas_id, &object.id),
        kind: KIND_OBJECT.to_string(),
        version,
        payload: serde_json::to_vec(object).expect("object serializes"),
    };
    (record, object_region_key(canvas_id, object))
}

fn meta_to_record(canvas_id: &CanvasId, scene: &ObjectScene, version: u64) -> Record {
    let meta = CanvasMeta {
        scene_version: scene.scene_version,
        tags: scene.tags.clone(),
        selection: scene.selection.clone(),
        updated_at: scene.updated_at.clone(),
    };
    Record {
        id: canvas_record_id(canvas_id),
        kind: KIND_CANVAS.to_string(),
        version,
        payload: serde_json::to_vec(&meta).expect("canvas-meta serializes"),
    }
}

/// Maps a malformed payload to a storage error (decode failure = backend corruption).
fn decode<T: for<'de> serde::Deserialize<'de>>(
    record: &Record,
) -> Result<T, shape_storage_core::StorageError> {
    serde_json::from_slice(&record.payload)
        .map_err(|e| shape_storage_core::StorageError::Serde(format!("record {}: {e}", record.id)))
}

/// Generic over the sync [`StorageAdapter`]; indexed-write and region-query
/// methods add a [`SpatialStore`] bound so they compile only for backends that
/// maintain a region index.
pub struct ObjectStore<A: StorageAdapter> {
    adapter: A,
    /// Per-canvas working set: the hydrated scene plus its LWW property gate.
    working: HashMap<String, (ObjectScene, PropertyStore)>,
}

impl<A: StorageAdapter> ObjectStore<A> {
    pub fn new(adapter: A) -> Self {
        ObjectStore { adapter, working: HashMap::new() }
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn into_adapter(self) -> A {
        self.adapter
    }

    /// Rebuild an [`ObjectScene`] from persisted records, objects id-sorted for a
    /// canonical result. No canvas-meta Record means scene-level defaults. The
    /// result is not cached — use [`ObjectStore::scene`] for the working-set scene.
    pub fn load_scene(&self, canvas_id: &CanvasId) -> Result<ObjectScene, ObjectStoreError> {
        let prefix = canvas_record_prefix(canvas_id);
        let canvas_rec_id = canvas_record_id(canvas_id);

        let mut objects: Vec<Object> = Vec::new();
        let mut meta: Option<CanvasMeta> = None;

        for id in self.adapter.list()? {
            if !id.starts_with(&prefix) {
                continue;
            }
            let record = self.adapter.load(&id)?;
            match record.kind.as_str() {
                KIND_OBJECT => {
                    let mut object: Object = decode(&record)?;
                    let _ = object.ensure_parsed();
                    objects.push(object);
                }
                KIND_CANVAS if id == canvas_rec_id => meta = Some(decode(&record)?),
                _ => {}
            }
        }

        objects.sort_by(|a, b| a.id.cmp(&b.id));

        let meta = meta.unwrap_or(CanvasMeta {
            scene_version: 0,
            tags: Vec::new(),
            selection: ObjectSelection::Canvas,
            updated_at: String::new(),
        });

        Ok(ObjectScene {
            scene_version: meta.scene_version,
            objects,
            tags: meta.tags,
            selection: meta.selection,
            updated_at: meta.updated_at,
        })
    }

    /// The cached working-set scene, loading + caching it on a cold canvas.
    pub fn scene(&mut self, canvas_id: &CanvasId) -> Result<&ObjectScene, ObjectStoreError> {
        self.ensure_loaded(canvas_id)?;
        Ok(&self.working.get(&canvas_id.0).expect("just loaded").0)
    }

    fn ensure_loaded(&mut self, canvas_id: &CanvasId) -> Result<(), ObjectStoreError> {
        if !self.working.contains_key(&canvas_id.0) {
            let scene = self.load_scene(canvas_id)?;
            self.working.insert(canvas_id.0.clone(), (scene, PropertyStore::new()));
        }
        Ok(())
    }
}

impl<A: StorageAdapter + SpatialStore> ObjectStore<A> {
    /// Write the object's `Record` + region row via the spatial store.
    pub fn save_object(
        &mut self,
        canvas_id: &CanvasId,
        object: &Object,
        version: u64,
    ) -> Result<(), ObjectStoreError> {
        let (record, region) = object_to_record(canvas_id, object, version);
        self.adapter.save_indexed(record, region)?;
        Ok(())
    }

    /// The canvas-meta Record carries no region key.
    pub fn save_meta(
        &mut self,
        canvas_id: &CanvasId,
        scene: &ObjectScene,
        version: u64,
    ) -> Result<(), ObjectStoreError> {
        let record = meta_to_record(canvas_id, scene, version);
        self.adapter.save(record)?;
        Ok(())
    }

    /// Apply `op` under server-authoritative per-property LWW at sequence `seq`,
    /// persisting the touched objects, and return the inverse for undo. A stale op
    /// (its `seq` lost the LWW race) applies as a no-op `Batch` and persists
    /// nothing; the canvas-meta is refreshed on every winning op so `sceneVersion`
    /// stays in step.
    pub fn apply(
        &mut self,
        canvas_id: &CanvasId,
        op: ObjectOp,
        seq: u64,
    ) -> Result<ObjectOp, ObjectStoreError> {
        self.ensure_loaded(canvas_id)?;
        let touched = op.target_ids();

        let (scene, store) = self.working.get_mut(&canvas_id.0).expect("just loaded");
        let inverse = apply_object_op_lww(scene, store, op, seq)?;

        // Stale op: scene untouched, inverse is an empty `Batch`, backend already
        // holds the winner — persist nothing.
        if inverse == (ObjectOp::Batch { ops: Vec::new() }) {
            return Ok(inverse);
        }

        // Snapshot before dropping the &mut borrow on `working`.
        let persist: Vec<(String, Option<Object>)> = touched
            .into_iter()
            .map(|id| (id.clone(), scene.get(&id).cloned()))
            .collect();
        let scene_for_meta = scene.clone();

        // Present -> upsert Record + region row; absent (a Delete) -> remove the
        // Record (and its region row).
        for (id, object) in persist {
            match object {
                Some(object) => self.save_object(canvas_id, &object, seq)?,
                None => {
                    self.adapter.delete(&object_record_id(canvas_id, &id))?;
                }
            }
        }
        self.save_meta(canvas_id, &scene_for_meta, seq)?;

        Ok(inverse)
    }

    /// Objects whose bbox overlaps `window` (`None` = whole canvas), id-sorted.
    /// Reads straight from the spatial backend (not the working set), which does
    /// the Morton/bbox refilter.
    pub fn query_region(
        &self,
        canvas_id: &CanvasId,
        window: Option<RegionWindow>,
    ) -> Result<Vec<Object>, ObjectStoreError> {
        let bbox = window.map(region_window_to_bbox);
        let mut objects = Vec::new();
        for record in self.adapter.query_region(&canvas_id.0, bbox)? {
            let record = record?;
            if record.kind == KIND_OBJECT {
                let mut object: Object = decode(&record)?;
                let _ = object.ensure_parsed();
                objects.push(object);
            }
        }
        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::object::{FillRule, PathNode, SubPath, Transform3x3};
    use shape_storage_core::MemoryAdapter;

    /// A closed rect at object-local (0,0)-(80,40) in quantized units.
    fn rect_geometry() -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(80, 0),
                    PathNode::corner(80, 40),
                    PathNode::corner(0, 40),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    fn rect_object(id: &str) -> Object {
        Object::new(id, "a0", rect_geometry())
    }

    #[test]
    fn region_key_is_world_aabb_under_translate() {
        let canvas = CanvasId::from("c-region");
        let mut obj = rect_object("r");
        obj.transform = Transform3x3::translate(100.0, 50.0);
        let key = object_region_key(&canvas, &obj).expect("placement object has a region");

        // Local extents 0..80 / 0..40 quantized => 0..10px / 0..5px, then +translate.
        assert_eq!(
            key,
            RegionKey { canvas_id: "c-region".to_string(), min_x: 100.0, min_y: 50.0, max_x: 110.0, max_y: 55.0 }
        );
    }

    #[test]
    fn empty_geometry_has_no_region_key() {
        let canvas = CanvasId::from("c-empty");
        let obj = Object::new("blank", "a0", Geometry::default());
        assert_eq!(object_region_key(&canvas, &obj), None);
    }

    #[test]
    fn insert_via_apply_then_load_scene_shows_it() {
        let canvas = CanvasId::from("c1");
        let mut store = ObjectStore::new(MemoryAdapter::new());

        let inverse = store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r1") }, 1)
            .expect("insert applies");
        assert_eq!(inverse, ObjectOp::Delete { id: "r1".into() });

        let scene = store.load_scene(&canvas).expect("load");
        assert_eq!(scene.objects.len(), 1);
        assert_eq!(scene.objects[0].id, "r1");
    }

    #[test]
    fn set_transform_via_apply_is_persisted() {
        let canvas = CanvasId::from("c2");
        let mut store = ObjectStore::new(MemoryAdapter::new());
        store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r1") }, 1)
            .expect("insert");

        store
            .apply(
                &canvas,
                ObjectOp::SetTransform { id: "r1".into(), transform: Transform3x3::translate(10.0, 20.0) },
                2,
            )
            .expect("set-transform");

        let scene = store.load_scene(&canvas).expect("load");
        assert_eq!(scene.objects[0].transform, Transform3x3::translate(10.0, 20.0));

        let rec = store
            .adapter()
            .load(&object_record_id(&canvas, "r1"))
            .expect("object record");
        let persisted: Object = serde_json::from_slice(&rec.payload).expect("decode");
        assert_eq!(persisted.transform, Transform3x3::translate(10.0, 20.0));
    }

    #[test]
    fn reload_from_fresh_store_on_same_adapter_shows_persistence() {
        let canvas = CanvasId::from("c3");
        let mut store = ObjectStore::new(MemoryAdapter::new());
        store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r1") }, 1)
            .expect("insert");
        store
            .apply(
                &canvas,
                ObjectOp::SetTransform { id: "r1".into(), transform: Transform3x3::translate(5.0, 5.0) },
                2,
            )
            .expect("move");

        let adapter = store.into_adapter();
        let reopened = ObjectStore::new(adapter);
        let scene = reopened.load_scene(&canvas).expect("reload");
        assert_eq!(scene.objects.len(), 1);
        assert_eq!(scene.objects[0].id, "r1");
        assert_eq!(scene.objects[0].transform, Transform3x3::translate(5.0, 5.0));
        // sceneVersion advanced once per winning op (insert + transform).
        assert_eq!(scene.scene_version, 2);
    }

    #[test]
    fn delete_via_apply_removes_object_record() {
        let canvas = CanvasId::from("c4");
        let mut store = ObjectStore::new(MemoryAdapter::new());
        store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r1") }, 1)
            .expect("insert");
        store
            .apply(&canvas, ObjectOp::Delete { id: "r1".into() }, 2)
            .expect("delete");

        let scene = store.load_scene(&canvas).expect("load");
        assert!(scene.objects.is_empty(), "deleted object is gone from the scene");
        assert!(store.adapter().load(&object_record_id(&canvas, "r1")).is_err());
    }

    #[test]
    fn query_region_windows_persisted_objects() {
        let canvas = CanvasId::from("c5");
        let mut store = ObjectStore::new(MemoryAdapter::new());

        // r-near at world (0..10, 0..5); r-far translated out to (1000..1010, 0..5).
        store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r-near") }, 1)
            .expect("insert near");
        let mut far = rect_object("r-far");
        far.transform = Transform3x3::translate(1000.0, 0.0);
        store
            .apply(&canvas, ObjectOp::InsertObject { object: far }, 2)
            .expect("insert far");

        let all = store.query_region(&canvas, None).expect("query all");
        let ids: Vec<&str> = all.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["r-far", "r-near"]);

        let near = store
            .query_region(
                &canvas,
                Some(RegionWindow { min_x: -5.0, min_y: -5.0, max_x: 20.0, max_y: 20.0 }),
            )
            .expect("query near");
        let near_ids: Vec<&str> = near.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(near_ids, vec!["r-near"]);
    }

    #[test]
    fn stale_op_is_a_noop_and_persists_nothing() {
        let canvas = CanvasId::from("c6");
        let mut store = ObjectStore::new(MemoryAdapter::new());
        store
            .apply(&canvas, ObjectOp::InsertObject { object: rect_object("r1") }, 5)
            .expect("insert");
        store
            .apply(
                &canvas,
                ObjectOp::SetTransform { id: "r1".into(), transform: Transform3x3::translate(10.0, 10.0) },
                10,
            )
            .expect("winning move");

        // A transform at an older seq loses the LWW race -> no-op Batch.
        let inverse = store
            .apply(
                &canvas,
                ObjectOp::SetTransform { id: "r1".into(), transform: Transform3x3::translate(99.0, 99.0) },
                7,
            )
            .expect("stale move applies as no-op");
        assert_eq!(inverse, ObjectOp::Batch { ops: Vec::new() });

        let scene = store.load_scene(&canvas).expect("load");
        assert_eq!(scene.objects[0].transform, Transform3x3::translate(10.0, 10.0));
    }
}
