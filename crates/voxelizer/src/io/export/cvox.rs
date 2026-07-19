//! `.cvox` output adapter (<https://github.com/JelleBouma/cvox>) — the
//! compressed sibling of [`super::vox`], and the export path free of that
//! format's two ceilings: colours are full RGBA (no 255-colour palette, so
//! truecolor scenes never quantize) and scenes larger than 256³ split into
//! multiple models with plain translations (which the import side re-merges).
//!
//! Compression: same-coloured voxels merge into inclusive-bound boxes via
//! sparse three-stage greedy meshing (x-runs → y-rects → z-boxes, per tile).
//! Multi-voxel boxes go to `CMAP`+`CUBE` (6 bytes each), 1×1×1 singles to
//! `VMAP`+`XYZ ` (3 bytes each), colour run-lengths in the maps. Colour byte
//! order is RGBA in both maps, matching the reference implementation (its
//! reader uses one RGBA parser for both; the spec's §7 ARGB note is the odd
//! one out).

use crate::error::VoxelizerError;

/// A merged axis-aligned box in tile-local coords, bounds inclusive.
#[derive(Clone, Copy)]
struct MergedBox {
    lo: [u8; 3],
    hi: [u8; 3],
    color: u32,
}

/// Serializes occupied voxels as a `.cvox` file.
///
/// `voxels` are `(engine-space coord, RGBA8 little-endian colour)` pairs; the
/// scene is cropped to their bounding box and split into 256³-tile models.
///
/// # Errors
/// [`VoxelizerError::MeshLoad`] when there are no voxels.
pub fn write_cvox_voxels(voxels: &[([u32; 3], u32)]) -> Result<Vec<u8>, VoxelizerError> {
    let Some(&(first, _)) = voxels.first() else {
        return Err(VoxelizerError::MeshLoad(
            "cvox export: scene has no occupied voxels".to_string(),
        ));
    };
    let (mut lo, mut hi) = (first, first);
    for &(c, _) in voxels {
        for a in 0..3 {
            lo[a] = lo[a].min(c[a]);
            hi[a] = hi[a].max(c[a]);
        }
    }
    // Crop, then rotate Y-up → Z-up (inverse of the import adapters):
    // (x, y, z)_cvox = (x, z_extent−1−z, y)_engine.
    let z_extent = hi[2] - lo[2];
    // Tiles keyed by their 256-aligned global origin; voxels tile-local.
    let mut tiles: std::collections::BTreeMap<[u32; 3], Vec<([u8; 3], u32)>> =
        std::collections::BTreeMap::new();
    for &(c, color) in voxels {
        let g = [c[0] - lo[0], z_extent - (c[2] - lo[2]), c[1] - lo[1]];
        let key = [g[0] & !255, g[1] & !255, g[2] & !255];
        let local = [
            u8::try_from(g[0] & 255).expect("masked to 8 bits"),
            u8::try_from(g[1] & 255).expect("masked to 8 bits"),
            u8::try_from(g[2] & 255).expect("masked to 8 bits"),
        ];
        tiles.entry(key).or_default().push((local, color));
    }

    let mut out = Vec::new();
    write_chunk(&mut out, *b"CVOX", &1u32.to_le_bytes());
    for (origin, tile) in &mut tiles {
        write_model(&mut out, *origin, tile);
    }
    Ok(out)
}

/// Emits one model: SIZE, then the cube pair and/or the singles pair.
fn write_model(out: &mut Vec<u8>, origin: [u32; 3], tile: &mut [([u8; 3], u32)]) {
    let boxes = merge_boxes(tile);
    let mut extent = [0u8; 3];
    for b in &boxes {
        for (e, &h) in extent.iter_mut().zip(b.hi.iter()) {
            *e = (*e).max(h);
        }
    }

    // SIZE: extents as bytes (a full 256 wraps to 0, mirroring the reference
    // writer's cast — readers derive extents from content) + the tile origin
    // as the translation.
    let mut size = Vec::with_capacity(15);
    size.extend_from_slice(&[
        extent[0].wrapping_add(1),
        extent[1].wrapping_add(1),
        extent[2].wrapping_add(1),
    ]);
    for o in origin {
        size.extend_from_slice(&o.to_le_bytes());
    }
    write_chunk(out, *b"SIZE", &size);

    // Colour-grouped, deterministically ordered runs for each geometry kind.
    let (mut cubes, mut singles): (Vec<MergedBox>, Vec<MergedBox>) =
        boxes.into_iter().partition(|b| b.lo != b.hi);
    cubes.sort_unstable_by_key(|b| (b.color, b.lo));
    singles.sort_unstable_by_key(|b| (b.color, b.lo));

    if !cubes.is_empty() {
        let (map, mut geo) = (run_lengths(&cubes), Vec::with_capacity(cubes.len() * 6));
        for b in &cubes {
            geo.extend_from_slice(&b.lo);
            geo.extend_from_slice(&b.hi);
        }
        write_chunk(out, *b"CMAP", &map);
        write_chunk(out, *b"CUBE", &geo);
    }
    if !singles.is_empty() {
        let (map, mut geo) = (run_lengths(&singles), Vec::with_capacity(singles.len() * 3));
        for b in &singles {
            geo.extend_from_slice(&b.lo);
        }
        write_chunk(out, *b"VMAP", &map);
        write_chunk(out, *b"XYZ ", &geo);
    }
}

/// The colour map for a colour-sorted box list: RGBA8-LE colour + 3-byte count.
fn run_lengths(sorted: &[MergedBox]) -> Vec<u8> {
    let mut map = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let color = sorted[i].color;
        let start = i;
        while i < sorted.len() && sorted[i].color == color {
            i += 1;
        }
        let count = u32::try_from(i - start).expect("tile holds < 2^24 boxes");
        debug_assert!(count < (1 << 24), "count must fit the 3-byte field");
        map.extend_from_slice(&color.to_le_bytes());
        map.extend_from_slice(&count.to_le_bytes()[..3]);
    }
    map
}

/// Sparse three-stage greedy meshing over one tile: sort, fuse consecutive-x
/// runs, fuse identical runs across y, fuse identical rects across z. Bounds
/// inclusive throughout, colours never mix.
fn merge_boxes(tile: &mut [([u8; 3], u32)]) -> Vec<MergedBox> {
    // Stage 1 — x-runs. Sort so voxels of a run are adjacent.
    tile.sort_unstable_by_key(|&(p, _)| (p[2], p[1], p[0]));
    let mut runs: Vec<MergedBox> = Vec::new();
    for &(p, color) in tile.iter() {
        match runs.last_mut() {
            Some(r)
                if r.color == color
                    && r.lo[1] == p[1]
                    && r.lo[2] == p[2]
                    && u16::from(r.hi[0]) + 1 == u16::from(p[0]) =>
            {
                r.hi[0] = p[0];
            }
            _ => runs.push(MergedBox {
                lo: p,
                hi: p,
                color,
            }),
        }
    }

    // Stage 2 — fuse runs with identical x-span/colour across consecutive y.
    runs.sort_unstable_by_key(|r| (r.lo[2], r.lo[0], r.hi[0], r.color, r.lo[1]));
    let mut rects: Vec<MergedBox> = Vec::new();
    for run in runs {
        match rects.last_mut() {
            Some(r)
                if r.color == run.color
                    && r.lo[2] == run.lo[2]
                    && r.lo[0] == run.lo[0]
                    && r.hi[0] == run.hi[0]
                    && u16::from(r.hi[1]) + 1 == u16::from(run.lo[1]) =>
            {
                r.hi[1] = run.hi[1];
            }
            _ => rects.push(run),
        }
    }

    // Stage 3 — fuse rects with identical xy-footprint/colour across
    // consecutive z.
    rects.sort_unstable_by_key(|r| (r.lo[0], r.hi[0], r.lo[1], r.hi[1], r.color, r.lo[2]));
    let mut boxes: Vec<MergedBox> = Vec::new();
    for rect in rects {
        match boxes.last_mut() {
            Some(b)
                if b.color == rect.color
                    && b.lo[0] == rect.lo[0]
                    && b.hi[0] == rect.hi[0]
                    && b.lo[1] == rect.lo[1]
                    && b.hi[1] == rect.hi[1]
                    && u16::from(b.hi[2]) + 1 == u16::from(rect.lo[2]) =>
            {
                b.hi[2] = rect.hi[2];
            }
            _ => boxes.push(rect),
        }
    }
    boxes
}

/// One RIFF-style chunk: 4-char id, 4-byte LE length, content.
fn write_chunk(out: &mut Vec<u8>, id: [u8; 4], content: &[u8]) {
    out.extend_from_slice(&id);
    out.extend_from_slice(
        &u32::try_from(content.len())
            .expect("chunk fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(content);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::import::cvox::load_cvox_slice;
    use voxel_core::Progress;

    /// Engine-space voxels → bytes → engine-space voxels: coordinates (cropped)
    /// and colours survive both axis rotations and the box merging.
    #[test]
    fn round_trip_preserves_voxels_and_colors() {
        let voxels: Vec<([u32; 3], u32)> = vec![
            ([10, 20, 30], 0xFF00_11FF),
            ([11, 20, 30], 0xFF00_11FF), // merges with the first into a 2-run
            ([10, 21, 33], 0xFF55_6677),
        ];
        let bytes = write_cvox_voxels(&voxels).expect("write");
        let model = load_cvox_slice(&bytes, &mut Progress::none()).expect("parse back");

        assert_eq!(model.models_total, 1);
        assert_eq!(model.dims, [2, 2, 4]);
        let mut got = model.voxels.clone();
        got.sort_unstable();
        let mut want: Vec<([u32; 3], u32)> = voxels
            .iter()
            .map(|&(c, col)| ([c[0] - 10, c[1] - 20, c[2] - 30], col))
            .collect();
        want.sort_unstable();
        assert_eq!(got, want);
    }

    /// A solid same-colour block merges into exactly one CUBE entry.
    #[test]
    fn solid_block_merges_to_one_cube() {
        let mut voxels = Vec::new();
        for x in 0..8u32 {
            for y in 0..8u32 {
                for z in 0..8u32 {
                    voxels.push(([x, y, z], 0xFFAA_BBCC));
                }
            }
        }
        let bytes = write_cvox_voxels(&voxels).expect("write");
        // One CUBE chunk with exactly one 6-byte entry.
        let cube_at = bytes
            .windows(4)
            .position(|w| w == b"CUBE")
            .expect("solid block must use the cube path");
        let len = u32::from_le_bytes(bytes[cube_at + 4..cube_at + 8].try_into().expect("4 bytes"));
        assert_eq!(len, 6, "8³ solid must merge to a single cube");

        let model = load_cvox_slice(&bytes, &mut Progress::none()).expect("parse back");
        assert_eq!(model.voxels.len(), 512);
    }

    /// Scenes wider than 256 split into tile models that re-merge on import —
    /// the size freedom `.vox` does not have.
    #[test]
    fn over_256_extent_splits_into_models_and_remerges() {
        let voxels = vec![
            ([0, 0, 0], 0xFF11_2233),
            ([300, 5, 7], 0xFF44_5566), // second 256-tile along x
        ];
        let bytes = write_cvox_voxels(&voxels).expect("write");
        let model = load_cvox_slice(&bytes, &mut Progress::none()).expect("parse back");
        assert_eq!(model.models_total, 2, "two x-tiles expected");
        assert_eq!(model.dims, [301, 6, 8]);
        let mut got = model.voxels.clone();
        got.sort_unstable();
        assert_eq!(
            got,
            vec![([0, 0, 0], 0xFF11_2233), ([300, 5, 7], 0xFF44_5566)]
        );
    }

    /// Isolated voxels take the 3-byte XYZ path, not the 6-byte cube path.
    #[test]
    fn singles_use_the_xyz_path() {
        let voxels = vec![([0, 0, 0], 0xFF01_0101), ([4, 4, 4], 0xFF02_0202)];
        let bytes = write_cvox_voxels(&voxels).expect("write");
        assert!(
            !bytes.windows(4).any(|w| w == b"CUBE"),
            "isolated voxels must not emit a CUBE chunk"
        );
        assert!(bytes.windows(4).any(|w| w == b"XYZ "));
        assert_eq!(
            load_cvox_slice(&bytes, &mut Progress::none())
                .expect("parse")
                .voxels
                .len(),
            2
        );
    }

    /// The truecolor regime: more distinct colours than a `MaterialTable`'s
    /// 65535 ids. The old palette-bridging loader errored here — on bytes this
    /// crate itself wrote; raw colours have no ceiling.
    #[test]
    fn round_trip_survives_more_colors_than_a_palette_holds() {
        let mut voxels = Vec::new();
        let mut i = 0u32;
        for x in 0..41u32 {
            for y in 0..40 {
                for z in 0..40 {
                    voxels.push(([x, y, z], 0xFF00_0000 | i)); // 65,600 distinct
                    i += 1;
                }
            }
        }
        let bytes = write_cvox_voxels(&voxels).expect("write");
        let model =
            load_cvox_slice(&bytes, &mut Progress::none()).expect("no colour ceiling on read");
        assert_eq!(model.voxels.len(), voxels.len());
        let mut got = model.voxels;
        got.sort_unstable();
        voxels.sort_unstable();
        assert_eq!(got, voxels, "every colour survives exactly");
    }

    #[test]
    fn empty_scene_is_a_typed_error() {
        assert!(write_cvox_voxels(&[]).is_err());
    }

    /// Malformed inputs are typed errors, not panics.
    #[test]
    fn malformed_bytes_are_typed_errors() {
        assert!(load_cvox_slice(b"nope", &mut Progress::none()).is_err());
        // Valid header, CMAP promising more cubes than CUBE holds.
        let mut bytes = Vec::new();
        write_chunk(&mut bytes, *b"CVOX", &1u32.to_le_bytes());
        let mut size = vec![1, 1, 1];
        size.extend_from_slice(&[0u8; 12]);
        write_chunk(&mut bytes, *b"SIZE", &size);
        let mut cmap = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
        cmap.extend_from_slice(&[2, 0, 0]); // two cubes promised
        write_chunk(&mut bytes, *b"CMAP", &cmap);
        write_chunk(&mut bytes, *b"CUBE", &[0, 0, 0, 0, 0, 0]); // one provided
        assert!(load_cvox_slice(&bytes, &mut Progress::none()).is_err());
    }
}
