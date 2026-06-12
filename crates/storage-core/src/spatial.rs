//! Optional region-query capability layered on the store-neutral [`Record`]
//! model without polluting it: spatial indexing is a side channel keyed by a
//! [`RegionKey`] (canvas id + AABB) that lives in a separate index, never inside
//! the record payload. The store itself stays domain-neutral.

use crate::adapter::RecordCursor;
use crate::error::Result;
use crate::record::Record;
use serde::{Deserialize, Serialize};

/// A record's spatial key: its canvas plus axis-aligned bounding box. Lives in a
/// side index, never inside the [`Record`] payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionKey {
    pub canvas_id: String,
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// Whether two bboxes overlap as an **inclusive** AABB intersection.
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

/// Optional capability on the store-neutral [`StorageAdapter`](crate::StorageAdapter):
/// only backends maintaining a region index implement it. `query_region` must
/// stay streaming and id-sorted, like [`records`](crate::StorageAdapter::records).
pub trait SpatialStore {
    /// Upsert `record`, and set (or, with `None`, clear) its region index row,
    /// atomically with respect to the record write.
    fn save_indexed(&mut self, record: Record, key: Option<RegionKey>) -> Result<()>;

    /// Stream the records of `canvas_id` in **id-sorted order**. With `Some`
    /// bbox, only records whose indexed bbox overlaps it (inclusive AABB
    /// intersect); `None` yields every indexed record on the canvas.
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

    /// A record on `canvas` with a square bbox centered at `(cx, cy)`, half `r`.
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

    /// Two-canvas fixture (alpha: 3 boxes along x; beta: 2), inserted out of id
    /// order so the cursor must impose order.
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

        assert_eq!(
            ids(store.query_region("alpha", None).unwrap()),
            vec!["a-10", "a-20", "a-30"]
        );
        assert_eq!(
            ids(store.query_region("beta", None).unwrap()),
            vec!["b-05", "b-15"]
        );

        // a-10 spans [5,15], a-20 [15,25], a-30 [25,35].
        assert_eq!(
            ids(store.query_region("alpha", Some((6.0, -1.0, 24.0, 1.0))).unwrap()),
            vec!["a-10", "a-20"]
        );

        // Inclusive AABB intersect: a window whose min_x is exactly 35 still
        // touches a-30's right edge ([25,35]) and nothing else.
        assert_eq!(
            ids(store
                .query_region("alpha", Some((35.0, -1.0, 40.0, 1.0)))
                .unwrap()),
            vec!["a-30"]
        );

        assert!(ids(store
            .query_region("alpha", Some((100.0, 100.0, 200.0, 200.0)))
            .unwrap())
        .is_empty());

        // beta's boxes sit in alpha's x range but never leak into an alpha query.
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

        // delete removes the record AND its region row.
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
            let mut sorted = m.clone();
            sorted.sort();
            assert_eq!(m, sorted, "memory result not id-sorted for {canvas}");
        }
    }
}
