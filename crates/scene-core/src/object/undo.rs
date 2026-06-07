//! OB3.S8 — per-actor undo/redo engine (D21).
//!
//! Undo is **reverse-op authoring**, never state rollback: to undo an edit the
//! caller re-applies its inverse op through the same `apply_object_op` path, so
//! undo composes with concurrent edits and the inverse is itself a normal op
//! (synced/persisted like any other). This engine owns no scene and performs no
//! apply — it is a pure bookkeeping stack. The host drives the apply and reports
//! the result back so the redo side stays consistent.
//!
//! # The re-inverse handshake
//!
//! `apply_object_op(op)` returns the inverse of whatever it just applied. So
//! applying an undo's inverse op yields the *re-inverse* — which is the op that
//! redoes the edit. The flow is therefore a two-step handshake per direction:
//!
//! ```text
//! let to_apply = stack.undo()?;              // inverse of the recorded forward
//! let re_inverse = apply_object_op(scene, to_apply)?;  // host applies it
//! stack.note_undo_applied(re_inverse);       // stages the redo entry
//! ```
//!
//! `note_undo_applied` cannot fail and must be called exactly once after a
//! successful apply; symmetrically for `redo` / `note_redo_applied`. Splitting
//! the apply out of the engine keeps the core pure (no scene access) while still
//! letting redo replay the freshly-derived re-inverse rather than a stale
//! pre-captured op (which matters once concurrent edits rebase the inverse).
//!
//! # Coalescing (gesture = 1 undo step, D21)
//!
//! A continuous gesture (e.g. many `set-transform` ops during a drag) must
//! collapse into one undo entry. The simplest correct rule: while a coalescing
//! window is open, the engine keeps the **first** inverse it ever saw (so undo
//! lands back at the pre-gesture state) and overwrites the **latest** forward
//! (so redo replays the gesture's final state). `begin_coalesce()` opens the
//! window; every `record` during it folds into the single live entry;
//! `end_coalesce()` closes it. `undo`/`redo` implicitly close any open window.

use super::op::ObjectOp;

/// One reversible step on a stack: the op that was applied (`forward`) and the
/// op that reverses it (`inverse`, as returned by `apply_object_op`).
#[derive(Clone, Debug, PartialEq)]
pub struct UndoEntry {
    /// The op that was applied to reach the current state.
    pub forward: ObjectOp,
    /// The op that, applied through `apply_object_op`, reverses `forward`.
    pub inverse: ObjectOp,
}

/// Which direction a handshake (`undo`/`redo`) is waiting on, so the matching
/// `note_*_applied` lands the entry on the correct opposite stack. Holds the
/// `forward` of the entry being moved; the re-inverse reported by the host
/// becomes the moved entry's `inverse`.
#[derive(Clone, Debug, PartialEq)]
enum Pending {
    /// `undo()` handed out an inverse; awaiting `note_undo_applied` to push the
    /// resulting redo entry. `forward` is the inverse op the host applied (i.e.
    /// the op that *redo* would have to reverse).
    Undo { forward: ObjectOp },
    /// `redo()` handed out an op; awaiting `note_redo_applied` to push the
    /// resulting undo entry. `forward` is the op the host applied (the original
    /// forward, modulo rebase).
    Redo { forward: ObjectOp },
}

/// Per-actor undo/redo stacks. Client-local session state — never synced; the
/// ops it hands out are applied through the normal (synced) op-apply path.
#[derive(Clone, Debug)]
pub struct UndoStack {
    actor_id: String,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    /// True while a gesture window is open (between `begin`/`end_coalesce`).
    coalescing_open: bool,
    /// The open window's live entry once its first edit has landed. `None` while
    /// the window is open but no edit has been recorded yet, so an empty gesture
    /// commits nothing.
    coalescing: Option<UndoEntry>,
    /// In-flight handshake awaiting a `note_*_applied`, if any.
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

    /// Record an applied edit. `forward` is what was applied; `inverse` is what
    /// `apply_object_op` returned for it. Clears the redo stack (a new edit
    /// forks history). During a coalescing window this folds into the single
    /// live entry instead of pushing a new one.
    pub fn record(&mut self, forward: ObjectOp, inverse: ObjectOp) {
        self.redo.clear();
        if self.coalescing_open {
            match &mut self.coalescing {
                // Fold into the live entry: keep the FIRST inverse (undo lands at
                // the pre-gesture state); overwrite with the LATEST forward (redo
                // replays the gesture's final state).
                Some(entry) => entry.forward = forward,
                // First edit of the gesture seeds the live entry.
                None => self.coalescing = Some(UndoEntry { forward, inverse }),
            }
        } else {
            self.undo.push(UndoEntry { forward, inverse });
        }
    }

    /// Open a coalescing window. The first `record` inside it seeds the live
    /// entry; subsequent records fold into it. No-op if already open.
    pub fn begin_coalesce(&mut self) {
        self.coalescing_open = true;
    }

    /// Close the coalescing window, committing its single entry (if any edits
    /// landed) onto the undo stack. No-op if no window is open.
    pub fn end_coalesce(&mut self) {
        self.coalescing_open = false;
        if let Some(entry) = self.coalescing.take() {
            self.undo.push(entry);
        }
    }

    /// Whether a coalescing window is currently open.
    pub fn is_coalescing(&self) -> bool {
        self.coalescing_open
    }

    /// Begin an undo: hand the caller the inverse op to apply through
    /// `apply_object_op`. The caller MUST then call [`note_undo_applied`] with
    /// the re-inverse that apply returned. Returns `None` when nothing to undo.
    /// Implicitly closes any open coalescing window first.
    ///
    /// [`note_undo_applied`]: Self::note_undo_applied
    pub fn undo(&mut self) -> Option<ObjectOp> {
        self.flush_coalesce();
        debug_assert!(self.pending.is_none(), "undo/redo handshake not completed");
        let entry = self.undo.pop()?;
        // The inverse is what the host applies; its re-inverse (reported back)
        // becomes the redo entry's inverse, with this entry's forward preserved.
        self.pending = Some(Pending::Undo { forward: entry.forward });
        Some(entry.inverse)
    }

    /// Complete the undo handshake. `re_inverse` is what `apply_object_op`
    /// returned when the caller applied the op from [`undo`]. Pushes the redo
    /// entry. Panics if no undo handshake is in flight.
    ///
    /// [`undo`]: Self::undo
    pub fn note_undo_applied(&mut self, re_inverse: ObjectOp) {
        match self.pending.take() {
            Some(Pending::Undo { forward }) => {
                // Redo will re-apply the original forward; undoing *that* again is
                // the re_inverse the host just derived.
                self.redo.push(UndoEntry { forward, inverse: re_inverse });
            }
            other => {
                self.pending = other;
                panic!("note_undo_applied without a pending undo");
            }
        }
    }

    /// Begin a redo: hand the caller the op to re-apply. The caller MUST then
    /// call [`note_redo_applied`] with the inverse that apply returned. Returns
    /// `None` when nothing to redo.
    ///
    /// [`note_redo_applied`]: Self::note_redo_applied
    pub fn redo(&mut self) -> Option<ObjectOp> {
        self.flush_coalesce();
        debug_assert!(self.pending.is_none(), "undo/redo handshake not completed");
        let entry = self.redo.pop()?;
        self.pending = Some(Pending::Redo { forward: entry.forward.clone() });
        Some(entry.forward)
    }

    /// Complete the redo handshake. `inverse` is what `apply_object_op` returned
    /// when the caller applied the op from [`redo`]. Pushes the undo entry back.
    /// Panics if no redo handshake is in flight.
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

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.coalescing.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of committed undo entries (excludes an open, unflushed gesture).
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Close any open coalescing window before an undo/redo crosses the gesture
    /// boundary, so the in-progress gesture becomes its single committed entry.
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

    /// A scene with one rect object `"r"` at the identity transform.
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

    /// record -> undo returns the captured inverse op (D21: undo hands out the
    /// inverse for the host to apply, not a rolled-back state).
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
        // The op handed out is exactly the inverse `apply_object_op` returned.
        assert_eq!(to_apply, inverse);
        assert_eq!(to_apply, ObjectOp::SetTransform {
            id: "r".into(),
            transform: Transform3x3::IDENTITY,
        });
    }

    /// Full round trip: undo restores prior state, and after the host applies the
    /// inverse + reports the re-inverse, redo replays the original forward and
    /// re-reaches the post-edit state.
    #[test]
    fn undo_then_redo_round_trips_through_apply() {
        let mut scene = scene_with_rect();
        let forward = set_transform("r", 5.0, 5.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        let after_edit = scene.get("r").unwrap().transform;
        assert_eq!(after_edit, Transform3x3::translate(5.0, 5.0));

        let mut stack = UndoStack::new("actor-1".into());
        stack.record(forward.clone(), inverse);

        // Undo: host applies the inverse op, scene returns to identity.
        let undo_op = stack.undo().expect("undo");
        let re_inverse = apply_object_op(&mut scene, undo_op).expect("apply undo");
        stack.note_undo_applied(re_inverse);
        assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);
        assert!(!stack.can_undo());
        assert!(stack.can_redo());
        assert_eq!(stack.redo_depth(), 1);

        // Redo: hands back the ORIGINAL forward; applying it re-reaches the edit.
        let redo_op = stack.redo().expect("redo");
        assert_eq!(redo_op, forward);
        let inv_again = apply_object_op(&mut scene, redo_op).expect("apply redo");
        stack.note_redo_applied(inv_again);
        assert_eq!(scene.get("r").unwrap().transform, after_edit);
        assert!(stack.can_undo());
        assert!(!stack.can_redo());
    }

    /// undo -> redo -> undo cycles indefinitely, staying consistent each pass.
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

    /// A new `record` after an undo forks history and clears the redo stack.
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

        // A brand-new edit must drop the redo history.
        let f2 = set_transform("r", 2.0, 0.0);
        let i2 = apply_object_op(&mut scene, f2.clone()).expect("apply");
        stack.record(f2, i2);
        assert!(!stack.can_redo());
        assert_eq!(stack.redo_depth(), 0);
    }

    /// A continuous drag — many set-transform records inside one coalescing
    /// window — collapses to ONE undo step that lands back at the pre-gesture
    /// state and redoes to the gesture's final state (D21).
    #[test]
    fn coalesced_drag_is_single_undo_step() {
        let mut scene = scene_with_rect();
        let mut stack = UndoStack::new("dragger".into());

        // Simulate a drag: a stream of absolute set-transform ops to growing
        // offsets. Each is applied to the scene and recorded inside the window.
        stack.begin_coalesce();
        assert!(stack.is_coalescing());
        for step in 1..=5_i64 {
            let tx = f64::from(i32::try_from(step).unwrap()) * 10.0;
            let forward = set_transform("r", tx, 0.0);
            let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
            stack.record(forward, inverse);
        }
        stack.end_coalesce();

        // The whole drag is exactly one undo entry.
        assert_eq!(stack.undo_depth(), 1);
        let final_transform = scene.get("r").unwrap().transform;
        assert_eq!(final_transform, Transform3x3::translate(50.0, 0.0));

        // One undo lands all the way back at the pre-gesture (identity) state,
        // because the entry kept the FIRST inverse (-> identity).
        let undo_op = stack.undo().expect("undo");
        let ri = apply_object_op(&mut scene, undo_op).expect("apply undo");
        stack.note_undo_applied(ri);
        assert_eq!(scene.get("r").unwrap().transform, Transform3x3::IDENTITY);

        // One redo replays the gesture's FINAL state, because the entry kept the
        // LATEST forward (-> translate 50).
        let redo_op = stack.redo().expect("redo");
        let inv = apply_object_op(&mut scene, redo_op).expect("apply redo");
        stack.note_redo_applied(inv);
        assert_eq!(scene.get("r").unwrap().transform, final_transform);
        assert_eq!(stack.undo_depth(), 1);
    }

    /// An empty coalescing window (begin/end with no records) commits nothing.
    #[test]
    fn empty_coalesce_window_commits_nothing() {
        let mut stack = UndoStack::new("a".into());
        stack.begin_coalesce();
        stack.end_coalesce();
        assert_eq!(stack.undo_depth(), 0);
        assert!(!stack.can_undo());
    }

    /// `undo`/`redo` implicitly flush an open gesture so they never straddle the
    /// boundary mid-window.
    #[test]
    fn undo_flushes_open_coalesce_window() {
        let mut scene = scene_with_rect();
        let mut stack = UndoStack::new("a".into());
        stack.begin_coalesce();
        let forward = set_transform("r", 7.0, 0.0);
        let inverse = apply_object_op(&mut scene, forward.clone()).expect("apply");
        stack.record(forward, inverse);
        // No explicit end_coalesce: undo() must flush it first, then act on it.
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
}
