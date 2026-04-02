//! GPU pipeline passes — compute and render.

pub mod build_indirect;
pub mod build_wireframe;
pub mod hiz_build;
pub mod mesh_count;
pub mod mesh_rebuild;
pub mod occlusion_cull;
pub mod prefix_sum;
pub mod summary;
