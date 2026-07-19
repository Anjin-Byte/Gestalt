//! `MagicaVoxel` `.vox` input adapter.
//!
//! Unlike the mesh adapters this is **voxel-native** IO: the file already
//! carries voxels + a 256-colour palette, so there is no `MeshInput` and no
//! voxelization pass — the output maps straight onto
//! [`SparseTree::from_voxels`](voxel_core::SparseTree::from_voxels) plus a
//! [`MaterialTable`] (the palette arm; no truecolor bake needed for ≤255
//! colours).
//!
//! Coordinate system: `.vox` is right-handed **Z-up**; this renderer is
//! right-handed **Y-up**. The adapter applies the proper rotation
//! `(x, y, z)_engine = (x, z, size_y − 1 − y)_vox` (determinant +1, no mirror).

use voxel_core::MaterialTable;

use super::VoxModel;
use crate::error::VoxelizerError;

/// Parses `MagicaVoxel` `.vox` bytes (the first model + palette).
///
/// # Errors
/// [`VoxelizerError::MeshLoad`] when the bytes are not a parseable `.vox` file,
/// the file has no models, or a voxel lies outside its model's declared size.
pub fn load_vox_slice(bytes: &[u8]) -> Result<VoxModel, VoxelizerError> {
    let data =
        dot_vox::load_bytes(bytes).map_err(|e| VoxelizerError::MeshLoad(format!("vox: {e}")))?;
    let model = data
        .models
        .first()
        .ok_or_else(|| VoxelizerError::MeshLoad("vox: file contains no models".to_string()))?;

    // Palette → table. Pushing every entry in order makes the mapping trivial:
    // dot_vox's 0-based palette index `i` lands at global id `i + 1` (id 0
    // stays the magenta MISSING sentinel). RGBA8 little-endian to match the
    // WGSL `unpack4x8unorm` contract.
    let mut table = MaterialTable::missing_only();
    for c in &data.palette {
        table
            .push(u32::from_le_bytes([c.r, c.g, c.b, c.a]))
            .map_err(|e| VoxelizerError::MeshLoad(format!("vox palette: {e}")))?;
    }

    // Z-up → Y-up (see module docs). `size.y` is the vox-space depth that
    // becomes the engine Z extent.
    let dims = [model.size.x, model.size.z, model.size.y];
    let flip = model.size.y;
    let mut voxels = Vec::with_capacity(model.voxels.len());
    for v in &model.voxels {
        let (vx, vy, vz) = (u32::from(v.x), u32::from(v.y), u32::from(v.z));
        if vx >= model.size.x || vy >= model.size.y || vz >= model.size.z {
            return Err(VoxelizerError::MeshLoad(format!(
                "vox: voxel ({vx},{vy},{vz}) outside declared size {:?}",
                (model.size.x, model.size.y, model.size.z)
            )));
        }
        let coord = [vx, vz, flip - 1 - vy];
        voxels.push((coord, u16::from(v.i) + 1));
    }

    Ok(VoxModel {
        voxels,
        table,
        dims,
        models_total: data.models.len(),
    })
}
