//! Spatial (region) query capability, layered on top of the store-neutral
//! [`Record`] model **without** polluting it.
//!
//! The storage core stays domain-neutral: a [`Record`] is still
//! `{id, kind, version, payload}` and knows nothing about geometry. Spatial
//! indexing is an *optional side channel* a backend can offer: a record can be
//! tagged with a [`RegionKey`] (a canvas id plus an axis-aligned bounding box)
//! that lives in a separate index, never inside the record's payload.
//!
//! [`SpatialStore`] is the capability:
//!
//! * [`save_indexed`](SpatialStore::save_indexed) upserts a record and,
//!   optionally, its region row in one step. Passing `None` clears any existing
//!   region row for that id (the record stays, just un-indexed).
//! * [`query_region`](SpatialStore::query_region) streams the records of one
//!   canvas in **id-sorted order**, optionally filtered to those whose bbox
//!   overlaps a query window. Overlap is an inclusive AABB intersection. A
//!   `None` window means "the whole canvas". The result is a streaming
//!   [`RecordCursor`] — bounded memory, one record resident at a time.

use crate::adapter::RecordCursor;
use crate::error::Result;
use crate::record::Record;
use serde::{Deserialize, Serialize};

/// A record's spatial key: which canvas it belongs to plus its axis-aligned
/// bounding box. Stored in a side index, never inside the [`Record`] payload,
/// so the data model stays domain-neutral.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionKey {
    /// The canvas this record lives on.
    pub canvas_id: String,
    /// Bounding box minimum x.
    pub min_x: f64,
    /// Bounding box minimum y.
    pub min_y: f64,
    /// Bounding box maximum x.
    pub max_x: f64,
    /// Bounding box maximum y.
    pub max_y: f64,
}

/// Whether bbox `(min_x,min_y,max_x,max_y)` overlaps the query window
/// `(qminx,qminy,qmaxx,qmaxy)` as an **inclusive** AABB intersection.
pub(crate) fn bbox_overlaps(
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    qminx: f64,
    qminy: f64,
    qmaxx: f64,
    qmaxy: f64,
) -> bool {
    min_x <= qmaxx && max_x >= qminx && min_y <= qmaxy && max_y >= qminy
}

/// A store that can index records spatially and answer region queries.
///
/// This is an *optional* capability bolted onto the store-neutral
/// [`StorageAdapter`](crate::StorageAdapter): only backends that maintain a
/// region index implement it. Implementations must keep
/// [`query_region`](SpatialStore::query_region) streaming and id-sorted, exactly
/// like [`records`](crate::StorageAdapter::records).
pub trait SpatialStore {
    /// Upsert `record`, and set (or, with `None`, clear) its region index row,
    /// atomically with respect to the record write.
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<()>;

    /// Stream the records of `canvas_id` in **id-sorted order**.
    ///
    /// `bbox` is an optional query window `(min_x, min_y, max_x, max_y)`; when
    /// `Some`, only records whose indexed bbox overlaps it (inclusive AABB
    /// intersect) are yielded. `None` yields every indexed record on the canvas.
    fn query_region(
        &self,
        canvas_id: &str,
        bbox: Option<(f64, f64, f64, f64)>,
    ) -> Result<RecordCursor<'_>>;
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::adapters::MemoryAdapter;
    use crate::StorageAdapter;

    /// A record on `canvas` with a square bbox centered at `(cx, cy)` and the
    /// given half-extent `r`.
    fn at(id: &str, canvas: &str, cx: f64, cy: f64, r: f64) -> (Record, RegionKey) {
        (
            Record::new(id, "card", id.as_bytes().to_vec()),
            RegionKey {
                canvas_id: canvas.to_string(),
                min_x: cx - r,
                min_y: cy - r,
                max_x: cx + r,
                max_y: cy + r,
            },
        )
    }

    /// Two-canvas fixture: canvas "alpha" has three boxes spread along x; canvas
    /// "beta" has two. Inserted out of id order so the cursor must impose order.
    fn fixture() -> Vec<(Record, RegionKey)> {
        vec![
            at("a-30", "alpha", 30.0, 0.0, 5.0),
            at("a-10", "alpha", 10.0, 0.0, 5.0),
            at("a-20", "alpha", 20.0, 0.0, 5.0),
            at("b-05", "beta", 5.0, 5.0, 2.0),
            at("b-15", "beta", 15.0, 15.0, 2.0),
        ]
    }

    fn ids(cursor: RecordCursor<'_>) -> Vec<String> {
        cursor.map(|r| r.unwrap().id).collect()
    }

    /// Drive the full region-query contract against any spatial store.
    fn run_contract<A: StorageAdapter + SpatialStore>(mut store: A) {
        for (record, key) in fixture() {
            store.save_indexed(record, Some(key)).unwrap();
        }

        // By-canvas isolation + whole-canvas (None bbox) completeness, id-sorted.
        assert_eq!(
            ids(store.query_region("alpha", None).unwrap()),
            vec!["a-10", "a-20", "a-30"]
        );
        assert_eq!(
            ids(store.query_region("beta", None).unwrap()),
            vec!["b-05", "b-15"]
        );

        // Window selecting only the alpha boxes near x in [6, 24] -> a-10, a-20.
        // (a-10 spans [5,15], a-20 spans [15,25], a-30 spans [25,35].)
        assert_eq!(
            ids(store.query_region("alpha", Some((6.0, -1.0, 24.0, 1.0))).unwrap()),
            vec!["a-10", "a-20"]
        );

        // Inclusive AABB intersect: touching an edge counts. a-30 spans x [25,35]
        // (a-20 ends at 25), so a thin window whose min_x is exactly 35 still
        // overlaps a-30's right edge and nothing else on the canvas.
        assert_eq!(
            ids(store
                .query_region("alpha", Some((35.0, -1.0, 40.0, 1.0)))
                .unwrap()),
            vec!["a-30"]
        );

        // A window far from everything on the canvas returns nothing.
        assert!(ids(store
            .query_region("alpha", Some((100.0, 100.0, 200.0, 200.0)))
            .unwrap())
        .is_empty());

        // The window must not leak across canvases: beta's boxes sit in alpha's x
        // range but querying alpha never returns them.
        let alpha_all = ids(store.query_region("alpha", None).unwrap());
        assert!(!alpha_all.iter().any(|id| id.starts_with("b-")));

        // save_indexed(None) clears the region row but keeps the record.
        let (rec, _) = at("a-20", "alpha", 20.0, 0.0, 5.0);
        store.save_indexed(rec, None).unwrap();
        assert_eq!(
            ids(store.query_region("alpha", None).unwrap()),
            vec!["a-10", "a-30"]
        );
        assert!(store.load("a-20").is_ok(), "record survives un-indexing");

        // delete removes the record AND its region row (no stale index entry).
        assert!(store.delete("a-10").unwrap());
        assert_eq!(ids(store.query_region("alpha", None).unwrap()), vec!["a-30"]);
    }

    #[test]
    fn memory_region_contract() {
        run_contract(MemoryAdapter::new());
    }

    #[cfg(feature = "redb")]
    #[test]
    fn redb_region_contract() {
        run_contract(crate::adapters::RedbAdapter::open_in_memory().unwrap());
    }

    /// redb and memory must return the *same id set* for the same query, across
    /// a varied multi-canvas dataset spanning more than one keyset page.
    #[cfg(feature = "redb")]
    #[test]
    fn redb_matches_memory() {
        let mut mem = MemoryAdapter::new();
        let mut redb = crate::adapters::RedbAdapter::open_in_memory().unwrap();

        // Enough records to cross the cursor's keyset page boundary.
        for i in 0..600usize {
            let canvas = ["alpha", "beta", "gamma"][i % 3];
            let cx = (i % 50) as f64 * 10.0;
            let cy = (i % 7) as f64 * 10.0;
            let (record, key) = at(&format!("n-{i:04}"), canvas, cx, cy, 3.0);
            mem.save_indexed(record.clone(), Some(key.clone())).unwrap();
            redb.save_indexed(record, Some(key)).unwrap();
        }

        let queries: [(&str, Option<(f64, f64, f64, f64)>); 5] = [
            ("alpha", None),
            ("beta", Some((0.0, 0.0, 100.0, 100.0))),
            ("gamma", Some((200.0, 0.0, 480.0, 60.0))),
            ("alpha", Some((-50.0, -50.0, -10.0, -10.0))), // empty window
            ("missing-canvas", None),                      // unknown canvas
        ];
        for (canvas, bbox) in queries {
            let m = ids(mem.query_region(canvas, bbox).unwrap());
            let r = ids(redb.query_region(canvas, bbox).unwrap());
            assert_eq!(m, r, "id set differs for query ({canvas}, {bbox:?})");
            // And both stay id-sorted.
            let mut sorted = m.clone();
            sorted.sort();
            assert_eq!(m, sorted, "memory result not id-sorted for {canvas}");
        }
    }
}
