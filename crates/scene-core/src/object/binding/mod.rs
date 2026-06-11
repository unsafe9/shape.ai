//! Binding tier — inter-object relations.
//!
//! Anchor endpoint resolution + the connection graph, follower reprojection,
//! transform cascades, the move-together binding graph, parent/child grouping,
//! auto-layout solve, and the derived outline/region contract.

pub mod anchor_follow;
pub mod anchors;
pub mod cascade;
pub mod grouping;
pub mod layout_solve;
pub mod move_together;
pub mod region;
