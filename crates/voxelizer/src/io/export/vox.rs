//! `MagicaVoxel` `.vox` output adapter — the first implemented arm of the export
//! side (the mesh-lowering direction stays deferred; see the parent module).
//!
//! Voxel-native: the input is `(coord, RGBA8-LE colour)` pairs read from the
//! voxel structure, so no geometry lowering is involved. The writer crops to
//! the occupied bounding box, quantizes to the format's 255-colour palette
//! (exact when ≤255 unique colours; weighted median-cut otherwise), applies
//! the inverse of the import adapter's Y-up→Z-up rotation, and serializes via
//! `dot_vox`.

use crate::error::VoxelizerError;

/// A `.vox` model is addressed with `u8` coordinates: 256 voxels per axis.
pub const VOX_MAX_EXTENT: u32 = 256;
/// Usable palette slots (index 0 is reserved by the format).
const VOX_MAX_COLORS: usize = 255;

/// Serializes occupied voxels as a single-model `.vox` file.
///
/// `voxels` are `(engine-space coord, RGBA8 little-endian colour)` pairs; the
/// model is cropped to their bounding box, so absolute placement in the source
/// grid does not matter.
///
/// # Errors
/// [`VoxelizerError::MeshLoad`] when there are no voxels or the occupied
/// bounding box exceeds [`VOX_MAX_EXTENT`] on any axis (`.vox` addresses
/// voxels with `u8` coordinates).
pub fn write_vox_voxels(voxels: &[([u32; 3], u32)]) -> Result<Vec<u8>, VoxelizerError> {
    let Some(&(first, _)) = voxels.first() else {
        return Err(VoxelizerError::MeshLoad(
            "vox export: scene has no occupied voxels".to_string(),
        ));
    };
    let (mut lo, mut hi) = (first, first);
    for &(c, _) in voxels {
        for a in 0..3 {
            lo[a] = lo[a].min(c[a]);
            hi[a] = hi[a].max(c[a]);
        }
    }
    let extent = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
    if extent.iter().any(|&e| e > VOX_MAX_EXTENT) {
        return Err(VoxelizerError::MeshLoad(format!(
            "vox export: occupied extent {extent:?} exceeds the format's {VOX_MAX_EXTENT}³ \
             model limit"
        )));
    }

    let (palette, index_of) = quantize_palette(voxels.iter().map(|&(_, c)| c));

    // Inverse of the import rotation: (x, y, z)_vox = (x, z_extent−1−z, y)_engine.
    let size = dot_vox::Size {
        x: extent[0],
        y: extent[2],
        z: extent[1],
    };
    let out_voxels: Vec<dot_vox::Voxel> = voxels
        .iter()
        .map(|&(c, color)| {
            let (ex, ey, ez) = (c[0] - lo[0], c[1] - lo[1], c[2] - lo[2]);
            dot_vox::Voxel {
                x: u8::try_from(ex).expect("cropped extent checked <= 256"),
                y: u8::try_from(extent[2] - 1 - ez).expect("cropped extent checked <= 256"),
                z: u8::try_from(ey).expect("cropped extent checked <= 256"),
                i: index_of(color),
            }
        })
        .collect();

    let data = dot_vox::DotVoxData {
        version: 150,
        index_map: Vec::new(),
        models: vec![dot_vox::Model {
            size,
            voxels: out_voxels,
        }],
        palette: palette
            .iter()
            .map(|&c| {
                let [r, g, b, a] = c.to_le_bytes();
                dot_vox::Color { r, g, b, a }
            })
            .collect(),
        materials: Vec::new(),
        scenes: Vec::new(),
        layers: Vec::new(),
    };
    let mut bytes = Vec::new();
    data.write_vox(&mut bytes)
        .map_err(|e| VoxelizerError::MeshLoad(format!("vox export: {e}")))?;
    Ok(bytes)
}

/// Builds a ≤255-entry palette for the colour stream and a colour→index map.
/// Exact passthrough when the unique colours fit; weighted median-cut on RGBA
/// otherwise. Returned indices are `dot_vox`'s 0-based palette positions.
fn quantize_palette(colors: impl Iterator<Item = u32>) -> (Vec<u32>, impl Fn(u32) -> u8) {
    let mut counts: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    for c in colors {
        *counts.entry(c).or_insert(0) += 1;
    }

    let palette: Vec<u32>;
    let mut index: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
    if counts.len() <= VOX_MAX_COLORS {
        palette = counts.keys().copied().collect();
        for (i, &c) in palette.iter().enumerate() {
            index.insert(c, u8::try_from(i).expect("<= 255 entries"));
        }
    } else {
        // Weighted median-cut: repeatedly split the box with the widest RGBA
        // channel range at its weighted median until 255 boxes exist, then
        // represent each box by its weighted mean colour.
        let mut boxes: Vec<Vec<(u32, u64)>> = vec![counts.iter().map(|(&c, &n)| (c, n)).collect()];
        while boxes.len() < VOX_MAX_COLORS {
            // Widest box: (box index, channel, range).
            let mut widest: Option<(usize, usize, u8)> = None;
            for (bi, b) in boxes.iter().enumerate() {
                if b.len() < 2 {
                    continue;
                }
                for ch in 0..4 {
                    let (mut mn, mut mx) = (u8::MAX, u8::MIN);
                    for &(c, _) in b {
                        let v = c.to_le_bytes()[ch];
                        mn = mn.min(v);
                        mx = mx.max(v);
                    }
                    let range = mx - mn;
                    if widest.is_none_or(|(_, _, w)| range > w) {
                        widest = Some((bi, ch, range));
                    }
                }
            }
            let Some((bi, ch, range)) = widest else {
                break; // every box is a single colour — nothing left to split
            };
            if range == 0 {
                break; // all remaining boxes are uniform
            }
            let mut b = std::mem::take(&mut boxes[bi]);
            b.sort_unstable_by_key(|&(c, _)| c.to_le_bytes()[ch]);
            let total: u64 = b.iter().map(|&(_, n)| n).sum();
            let mut acc = 0u64;
            let mut split = b.len() - 1; // at least one element stays on the right
            for (i, &(_, n)) in b.iter().enumerate() {
                acc += n;
                if acc * 2 >= total {
                    split = (i + 1).min(b.len() - 1);
                    break;
                }
            }
            let right = b.split_off(split.max(1));
            boxes[bi] = b;
            boxes.push(right);
        }
        palette = boxes
            .iter()
            .map(|b| {
                let total: u64 = b.iter().map(|&(_, n)| n).sum::<u64>().max(1);
                let mut mean = [0u64; 4];
                for &(c, n) in b {
                    for (m, &byte) in mean.iter_mut().zip(c.to_le_bytes().iter()) {
                        *m += u64::from(byte) * n;
                    }
                }
                u32::from_le_bytes(
                    mean.map(|m| u8::try_from(m / total).expect("mean of u8 values fits u8")),
                )
            })
            .collect();
        for (bi, b) in boxes.iter().enumerate() {
            for &(c, _) in b {
                index.insert(c, u8::try_from(bi).expect("<= 255 boxes"));
            }
        }
    }

    (palette, move |c: u32| {
        index.get(&c).copied().unwrap_or(0) // unreachable: every input colour was counted
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::import::vox::load_vox_slice;

    /// Write → parse round-trip preserves the voxel set and exact colours
    /// (≤255 unique → passthrough palette).
    #[test]
    fn round_trip_preserves_voxels_and_colors() {
        let voxels: Vec<([u32; 3], u32)> = vec![
            ([10, 20, 30], 0xFF00_11FF),
            ([11, 20, 30], 0xFF22_3344),
            ([10, 21, 33], 0xFF55_6677),
        ];
        let bytes = write_vox_voxels(&voxels).expect("write");
        let model = load_vox_slice(&bytes).expect("parse back");

        assert_eq!(model.voxels.len(), 3);
        // Cropped extents: x 10..=11, y 20..=21, z 30..=33 → [2, 2, 4].
        assert_eq!(model.dims, [2, 2, 4]);
        // Rebuild (cropped coord → colour) from the round-trip and compare.
        let mut got: Vec<([u32; 3], u32)> = model
            .voxels
            .iter()
            .map(|&(c, gid)| (c, model.table.color(gid)))
            .collect();
        got.sort_unstable();
        let mut want: Vec<([u32; 3], u32)> = voxels
            .iter()
            .map(|&(c, col)| ([c[0] - 10, c[1] - 20, c[2] - 30], col))
            .collect();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    /// Over-255 unique colours quantize to a legal palette and every voxel
    /// still maps to a palette entry.
    #[test]
    fn quantizes_over_255_colors() {
        let voxels: Vec<([u32; 3], u32)> = (0..300u32)
            .map(|i| {
                let c = 0xFF00_0000 | (i * 7919) & 0x00FF_FFFF;
                ([i % 16, (i / 16) % 16, i / 256], c)
            })
            .collect();
        let bytes = write_vox_voxels(&voxels).expect("write quantized");
        let model = load_vox_slice(&bytes).expect("parse back");
        assert_eq!(model.voxels.len(), 300);
        // Every voxel's id resolves to a non-sentinel palette colour.
        for &(_, gid) in &model.voxels {
            assert_ne!(gid, 0, "quantized voxel lost its palette entry");
        }
    }

    #[test]
    fn oversized_extent_is_a_typed_error() {
        let voxels = vec![([0, 0, 0], 0xFFFF_FFFF), ([300, 0, 0], 0xFFFF_FFFF)];
        assert!(write_vox_voxels(&voxels).is_err());
    }

    #[test]
    fn empty_scene_is_a_typed_error() {
        assert!(write_vox_voxels(&[]).is_err());
    }
}
