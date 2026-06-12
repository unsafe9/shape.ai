//! Per-actor undo/redo engine. Undo is reverse-op authoring, never state
//! rollback: the host re-applies the inverse op through `apply_object_op`. This
//! engine owns no scene and performs no apply — a pure bookkeeping stack.
//!
//! Re-inverse handshake: `apply_object_op(op)` returns the inverse of what it
//! applied, so applying an undo's inverse yields the re-inverse (the redo op).
//! `undo()`/`redo()` hand out an op; the host applies it and reports the result
//! via the matching `note_*_applied` (exactly once). Keeping apply out of the
//! engine lets redo replay the freshly-derived re-inverse rather than a stale
//! pre-captured op (which matters once concurrent edits rebase the inverse).
//!
//! Coalescing: a gesture collapses to one undo entry — within the window the
//! engine keeps the FIRST inverse (undo lands at the pre-gesture state) and the
//! LATEST forward (redo replays the gesture's final state). `undo`/`redo`
//! implicitly close any open window.

use crate::object::op::ObjectOp;

#[derive(Clone, Debug, PartialEq)]
pub struct UndoEntry {
    pub forward: ObjectOp,
    pub inverse: ObjectOp,
}

/// Which direction a handshake is waiting on, so the matching `note_*_applied`
/// lands the entry on the correct opposite stack. Holds the `forward` of the
/// entry being moved; the host-reported re-inverse becomes its `inverse`.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    Undo { forward: ObjectOp },
    Redo { forward: ObjectOp },
}

/// Client-local session state — never synced; the ops it hands out are applied
/// through the normal (synced) op-apply path.
#[derive(Clone, Debug)]
pub struct UndoStack {
    actor_id: String,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    coalescing_open: bool,
    /// `None` while the window is open but no edit has been recorded, so an empty
    /// gesture commits nothing.
    coalescing: Option<UndoEntry>,
    pending: Option<Pending>,
}

impl UndoStack {
    pub fn new(actor_id: String) -> Self {
        Self {
            actor_id,
            undo: Vec::new(),
            redo: Vec::new(),
            coalescing_open: false,
            coalescing: None,
            pending: None,
        }
    }

    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    /// Clears the redo stack (a new edit forks history). During a coalescing
    /// window this folds into the single live entry.
    pub fn record(&mut self, forward: ObjectOp, inverse: ObjectOp) {
        self.redo.clear();
        if self.coalescing_open {
            match &mut self.coalescing {
                // Keep the FIRST inverse, overwrite with the LATEST forward.
                Some(entry) => entry.forward = forward,
                None => self.coalescing = Some(UndoEntry { forward, inverse }),
            }
        } else {
            self.undo.push(UndoEntry { forward, inverse });
        }
    }

    /// No-op if already open.
    pub fn begin_coalesce(&mut self) {
        self.coalescing_open = true;
    }

    /// Commit the window's single entry (if any) onto the undo stack. No-op if no
    /// window is open.
    pub fn end_coalesce(&mut self) {
        self.coalescing_open = false;
        if let Some(entry) = self.coalescing.take() {
            self.undo.push(entry);
        }
    }

    pub fn is_coalescing(&self) -> bool {
        self.coalescing_open
    }

    /// Hand the caller the inverse op to apply; they MUST then call
    /// [`note_undo_applied`] with the re-inverse apply returned. `None` when
    /// nothing to undo. Implicitly closes any open coalescing window.
    ///
    /// [`note_undo_applied`]: Self::note_undo_applied
    pub fn undo(&mut self) -> Option<ObjectOp> {
        self.flush_coalesce();
        debug_assert!(self.pending.is_none(), "undo/redo handshake not completed");
        let entry = self.undo.pop()?;
        self.pending = Some(Pending::Undo { forward: entry.forward });
        Some(entry.inverse)
    }

    /// `re_inverse` is what apply returned for the op from [`undo`]. Panics if no
    /// undo handshake is in flight.
    ///
    /// [`undo`]: Self::undo
    pub fn note_undo_applied(&mut self, re_inverse: ObjectOp) {
        match self.pending.take() {
            Some(Pending::Undo { forward }) => {
                self.redo.push(UndoEntry { forward, inverse: re_inverse });
            }
            other => {
                self.pending = other;
                panic!("note_undo_applied without a pending undo");
            }
        }
    }

    /// Hand the caller the op to re-apply; they MUST then call
    /// [`note_redo_applied`] with the inverse apply returned. `None` when nothing
    /// to redo.
    ///
    /// [`note_redo_applied`]: Self::note_redo_applied
    pub fn redo(&mut self) -> Option<ObjectOp> {
        self.flush_coalesce();
        debug_assert!(self.pending.is_none(), "undo/redo handshake not completed");
        let entry = self.redo.pop()?;
        self.pending = Some(Pending::Redo { forward: entry.forward.clone() });
        Some(entry.forward)
    }

    /// `inverse` is what apply returned for the op from [`redo`]. Panics if no
    /// redo handshake is in flight.
    ///
    /// [`redo`]: Self::redo
    pub fn note_redo_applied(&mut self, inverse: ObjectOp) {
        match self.pending.take() {
            Some(Pending::Redo { forward }) => {
                self.undo.push(UndoEntry { forward, inverse });
            }
            other => {
                self.pending = other;
                panic!("note_redo_applied without a pending redo");
            }
        }
    }

    /// Drop an in-flight handshake when the host's apply did not succeed; a failed
    /// apply left the scene unchanged, so there is nothing to roll back.
    pub fn abort_pending(&mut self) {
        self.pending = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.coalescing.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Committed undo entries (excludes an open, unflushed gesture).
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    fn flush_coalesce(&mut self) {
        if self.coalescing_open {
            self.end_coalesce();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::apply::apply_object_op;
    use crate::object::model::{
        FillRule, Geometry, Object, ObjectScene, PathNode, SubPath, Transform3x3,
    };
    use crate::object::op::ObjectOp;

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

    fn scene_with_rect() -> ObjectScene {
        let mut scene = ObjectScene::default();
        apply_object_op(
            &mut scene,
            ObjectOp::InsertObject { object: Object::new("r", "a0", rect_geometry()) },
        )
        .expect("insert");
        scene
    }

    fn set_transform(id: &str, tx: f64, ty: f64) -> ObjectOp {
        ObjectOp::SetTransform { id: id.into(), transform: Transform3x3::translate(tx, ty) }
    }

    #[test]
    fn record_then_undo_returns_inverse() {
        let mut scene = scene_with_rect();
        let forward = set_transform("r", 5.0, 5.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");

        let mut stack = UndoStack::new("actor-1".into());
        stack.record(forward.clone(), inverse.clone());
        assert!(stack.can_undo());
        assert_eq!(stack.undo_depth(), 1);

        let to_apply = stack.undo().expect("undo available");
        assert_eq!(to_apply, inverse);
        assert_eq!(to_apply, ObjectOp::SetTransform {
            id: "r".into(),
            transform: Transform3x3::IDENTITY,
        });
    }

    #[test]
    fn undo_then_redo_round_trips_through_apply() {
        let mut scene = scene_with_rect();
        let forward = set_transform("r", 5.0, 5.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        let after_edit = scene.get("r").unwrap().transform;
        assert_eq!(after_edit, Transform3x3::translate(5.0, 5.0));

        let mut stack = UndoStack::new("actor-1".into());
        stack.record(forward.clone(), inverse);

        let undo_op = stack.undo().expect("undo");
        let re_inverse = apply_object_op(&mut scene, undo_op).expect("apply undo");
        stack.note_undo_applied(re_inverse);
        assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);
        assert!(!stack.can_undo());
        assert!(stack.can_redo());
        assert_eq!(stack.redo_depth(), 1);

        let redo_op = stack.redo().expect("redo");
        assert_eq!(redo_op, forward);
        let inv_again = apply_object_op(&mut scene, redo_op).expect("apply redo");
        stack.note_redo_applied(inv_again);
        assert_eq!(scene.get("r").unwrap().transform, after_edit);
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn repeated_undo_redo_stays_consistent() {
        let mut scene = scene_with_rect();
        let forward = set_transform("r", 9.0, 0.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        let mut stack = UndoStack::new("actor-1".into());
        stack.record(forward.clone(), inverse);

        for _ in 0..3 {
            let undo_op = stack.undo().expect("undo");
            let ri = apply_object_op(&mut scene, undo_op).expect("apply");
            stack.note_undo_applied(ri);
            assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);

            let redo_op = stack.redo().expect("redo");
            assert_eq!(redo_op, forward);
            let inv = apply_object_op(&mut scene, redo_op).expect("apply");
            stack.note_redo_applied(inv);
            assert_eq!(scene.get("r").unwrap().transform, Transform3x3::translate(9.0, 0.0));
        }
    }

    #[test]
    fn record_clears_redo() {
        let mut scene = scene_with_rect();
        let f1 = set_transform("r", 1.0, 0.0);
        let i1 = apply_object_op(&mut scene, f1.clone()).expect("apply");
        let mut stack = UndoStack::new("a".into());
        stack.record(f1, i1);

        let undo_op = stack.undo().expect("undo");
        let ri = apply_object_op(&mut scene, undo_op).expect("apply");
        stack.note_undo_applied(ri);
        assert!(stack.can_redo());

        let f2 = set_transform("r", 2.0, 0.0);
        let i2 = apply_object_op(&mut scene, f2.clone()).expect("apply");
        stack.record(f2, i2);
        assert!(!stack.can_redo());
        assert_eq!(stack.redo_depth(), 0);
    }

    #[test]
    fn coalesced_drag_is_single_undo_step() {
        let mut scene = scene_with_rect();
        let mut stack = UndoStack::new("dragger".into());

        stack.begin_coalesce();
        assert!(stack.is_coalescing());
        for step in 1..=5_i64 {
            let tx = f64::from(i32::try_from(step).unwrap()) * 10.0;
            let forward = set_transform("r", tx, 0.0);
            let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
            stack.record(forward, inverse);
        }
        stack.end_coalesce();

        assert_eq!(stack.undo_depth(), 1);
        let final_transform = scene.get("r").unwrap().transform;
        assert_eq!(final_transform, Transform3x3::translate(50.0, 0.0));

        // One undo lands at the pre-gesture (identity) state (kept FIRST inverse).
        let undo_op = stack.undo().expect("undo");
        let ri = apply_object_op(&mut scene, undo_op).expect("apply undo");
        stack.note_undo_applied(ri);
        assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);

        // One redo replays the gesture's final state (kept LATEST forward).
        let redo_op = stack.redo().expect("redo");
        let inv = apply_object_op(&mut scene, redo_op).expect("apply redo");
        stack.note_redo_applied(inv);
        assert_eq!(scene.get("r").unwrap().transform, final_transform);
        assert_eq!(stack.undo_depth(), 1);
    }

    #[test]
    fn empty_coalesce_window_commits_nothing() {
        let mut stack = UndoStack::new("a".into());
        stack.begin_coalesce();
        stack.end_coalesce();
        assert_eq!(stack.undo_depth(), 0);
        assert!(!stack.can_undo());
    }

    #[test]
    fn undo_flushes_open_coalesce_window() {
        let mut scene = scene_with_rect();
        let mut stack = UndoStack::new("a".into());
        stack.begin_coalesce();
        let forward = set_transform("r", 7.0, 0.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        stack.record(forward, inverse);
        let undo_op = stack.undo().expect("undo flushes then pops the gesture");
        assert!(!stack.is_coalescing());
        let ri = apply_object_op(&mut scene, undo_op).expect("apply");
        stack.note_undo_applied(ri);
        assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);
    }

    #[test]
    #[should_panic(expected = "note_undo_applied without a pending undo")]
    fn note_undo_applied_without_handshake_panics() {
        let mut stack = UndoStack::new("a".into());
        stack.note_undo_applied(set_transform("r", 0.0, 0.0));
    }

    #[test]
    fn undo_redo_empty_returns_none() {
        let mut stack = UndoStack::new("a".into());
        assert!(stack.undo().is_none());
        assert!(stack.redo().is_none());
    }

    #[test]
    fn abort_pending_clears_handshake_so_next_undo_is_reusable() {
        let mut scene = scene_with_rect();
        let mut stack = UndoStack::new("a".into());
        let forward = set_transform("r", 7.0, 0.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        stack.record(forward, inverse);
        let _op = stack.undo().expect("undo hands out the inverse");
        stack.abort_pending();
        // A fresh undo must not panic on the leftover pending; the aborted entry
        // was consumed.
        assert!(!stack.can_undo());
        assert!(stack.undo().is_none());
    }
}
