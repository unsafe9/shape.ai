//! Selection queries over a live scene. `Multi` is ephemeral shell-only; the
//! canonical collapse rule (declared on [`ObjectSelection`] in `model.rs`) is the
//! single source of truth the shell mirrored in TS, lifted here so no shell
//! re-derives it.

use crate::object::model::{ObjectScene, ObjectSelection};

/// Reconcile a selection against the current scene: drop ids no longer present
/// and collapse the kind as the live set shrinks. The collapse rule is
/// `>=2 live -> multi`, `1 -> object`, `0 -> canvas`; a `canvas` selection is
/// always valid, and a single `object` stays only while its id is live.
pub fn valid_selection(scene: &ObjectScene, selection: &ObjectSelection) -> ObjectSelection {
    match selection {
        ObjectSelection::Canvas => ObjectSelection::Canvas,
        ObjectSelection::Object { id } => {
            if scene.objects.iter().any(|o| &o.id == id) {
                ObjectSelection::Object { id: id.clone() }
            } else {
                ObjectSelection::Canvas
            }
        }
        ObjectSelection::Multi { ids } => {
            let live: Vec<_> = ids
                .iter()
                .filter(|id| scene.objects.iter().any(|o| &o.id == *id))
                .cloned()
                .collect();
            collapse(live)
        }
    }
}

/// Select every object in the scene, collapsed by the same rule as
/// [`valid_selection`]: an empty scene is `canvas`, a one-object scene is
/// `object`, otherwise `multi`.
pub fn select_all(scene: &ObjectScene) -> ObjectSelection {
    collapse(scene.objects.iter().map(|o| o.id.clone()).collect())
}

/// Collapse a live id set into the canonical selection kind.
fn collapse(ids: Vec<String>) -> ObjectSelection {
    match ids.len() {
        0 => ObjectSelection::Canvas,
        1 => ObjectSelection::Object { id: ids.into_iter().next().expect("len==1") },
        _ => ObjectSelection::Multi { ids },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::model::{FillRule, Geometry, Object, PathNode, SubPath};

    fn rect() -> Geometry {
        Geometry::from_subpaths(
            vec![SubPath {
                closed: true,
                nodes: vec![
                    PathNode::corner(0, 0),
                    PathNode::corner(10, 0),
                    PathNode::corner(10, 10),
                    PathNode::corner(0, 10),
                ],
            }],
            FillRule::EvenOdd,
        )
    }

    fn scene_with(ids: &[&str]) -> ObjectScene {
        let mut scene = ObjectScene::default();
        for (n, id) in ids.iter().enumerate() {
            scene.objects.push(Object::new(*id, &format!("a{n}"), rect()));
        }
        scene
    }

    // The collapse table, driven by the real query: as the live set shrinks the
    // kind degrades multi -> object -> canvas.
    #[test]
    fn valid_selection_collapses_multi_by_live_count() {
        let scene = scene_with(&["a", "b", "c"]);

        // 2+ live ids stay multi (and stale ids are dropped).
        let sel = ObjectSelection::Multi { ids: vec!["a".into(), "b".into(), "gone".into()] };
        assert_eq!(
            valid_selection(&scene, &sel),
            ObjectSelection::Multi { ids: vec!["a".into(), "b".into()] }
        );

        // Exactly 1 live id collapses to object.
        let sel = ObjectSelection::Multi { ids: vec!["a".into(), "gone".into()] };
        assert_eq!(
            valid_selection(&scene, &sel),
            ObjectSelection::Object { id: "a".into() }
        );

        // 0 live ids collapse to canvas.
        let sel = ObjectSelection::Multi { ids: vec!["gone".into(), "also-gone".into()] };
        assert_eq!(valid_selection(&scene, &sel), ObjectSelection::Canvas);
    }

    #[test]
    fn valid_selection_drops_stale_object_to_canvas() {
        let scene = scene_with(&["a"]);
        assert_eq!(
            valid_selection(&scene, &ObjectSelection::Object { id: "a".into() }),
            ObjectSelection::Object { id: "a".into() }
        );
        assert_eq!(
            valid_selection(&scene, &ObjectSelection::Object { id: "gone".into() }),
            ObjectSelection::Canvas
        );
    }

    #[test]
    fn select_all_collapses_by_object_count() {
        assert_eq!(select_all(&scene_with(&[])), ObjectSelection::Canvas);
        assert_eq!(
            select_all(&scene_with(&["only"])),
            ObjectSelection::Object { id: "only".into() }
        );
        assert_eq!(
            select_all(&scene_with(&["a", "b"])),
            ObjectSelection::Multi { ids: vec!["a".into(), "b".into()] }
        );
    }
}
