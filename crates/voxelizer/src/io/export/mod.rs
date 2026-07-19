//! Output adapters — the mirror of [`super::import`].
//!
//! The export source-of-truth is the voxel structure (`voxel_core::SparseTree`
//! / `SchoolBBuffer` / `MaterialTable`), read as output adapters parallel to
//! import.
//!
//! # Format modules
//! - [`vox`] — `MagicaVoxel`, gated behind the `vox` feature. **Voxel-native**
//!   (voxels + palette serialize directly), so it needs none of the deferred
//!   mesh machinery below.
//!
//! # Still deferred: mesh export
//! Lowering voxels back to a *mesh* format (voxel-cubes, then re-mesh, feeding
//! a `MeshOutput` DTO that per-format writers serialize) remains spec-only —
//! see `docs/materials/11` and the IO-boundary design notes for the deferral
//! rationale and the `voxel-io` extraction triggers.

#[cfg(feature = "cvox")]
pub mod cvox;
#[cfg(feature = "vox")]
pub mod vox;

#[cfg(feature = "cvox")]
pub use cvox::write_cvox_voxels;
#[cfg(feature = "vox")]
pub use vox::{VOX_MAX_EXTENT, write_vox_voxels};
