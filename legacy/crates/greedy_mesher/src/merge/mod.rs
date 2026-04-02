//! Greedy merge algorithms for each face direction.
//!
//! The greedy merge algorithm combines adjacent faces with the same material
//! into larger quads, significantly reducing triangle count.
//!
//! Each direction has a specialized implementation due to different axis mappings:
//! - Y faces: sweep through XZ slices, merge in X then Z
//! - X faces: sweep through YZ slices, merge in Y then Z
//! - Z faces: sweep through XY slices, merge in X then Y

mod y_faces;
mod x_faces;
mod z_faces;

pub use y_faces::greedy_merge_y_faces;
pub use x_faces::greedy_merge_x_faces;
pub use z_faces::greedy_merge_z_faces;
