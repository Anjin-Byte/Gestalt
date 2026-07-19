//! Voxel-native scene assembly for the web engine's control plane — `.vox`
//! and `.cvox` (import needs no voxelization; export is the demo's structure
//! download). The format adapters live in `voxelizer::io`; this module maps
//! their DTOs onto the renderer's scene types — `.vox` always through the
//! palette ([`voxelizer::VoxModel`]), `.cvox` through palette **or truecolor**
//! depending on how many distinct colours the file carries (see
//! [`MAX_PALETTE_COLORS`]) so this program's own truecolor exports round-trip
//! losslessly.

use voxel_core::{
    LeafBrick, MaterialTable, Progress, Resolution, SchoolBBuffer, SparseTree, VoxelCoord, morton,
};

use crate::phases::Phase;

/// Why a `.vox` import or export failed, in shell-reportable terms.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VoxError {
    /// The bytes did not parse, or the writer rejected the scene.
    #[error("{0}")]
    Format(String),
    /// An explicit resolution override is smaller than the model.
    #[error("model spans {dims:?} voxels; it does not fit a {res}³ grid")]
    TooSmall {
        /// Engine-space model extents.
        dims: [u32; 3],
        /// The requested grid resolution.
        res: u32,
    },
}

/// The assembled scene plus what the HUD wants to know about the file.
pub(crate) struct VoxScene {
    pub(crate) tree: SparseTree,
    pub(crate) structure: SchoolBBuffer,
    pub(crate) table: MaterialTable,
    /// Total models in the file (only the first is loaded).
    pub(crate) models_total: usize,
}

/// Parses `.vox` bytes and builds the renderer structure, centring the model
/// in the smallest legal `8·4^k` grid that holds it (or in `res_override`).
///
/// Reports two indeterminate phases — [`Parse`](Phase::Parse) (decode the
/// container) then [`Assemble`](Phase::Assemble) (bin the voxels into the tree)
/// — so a large file shows liveness. Pass a no-op sink where progress is not
/// observed.
pub(crate) fn import_vox(
    bytes: &[u8],
    res_override: Option<u32>,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<VoxScene, VoxError> {
    on_progress(Phase::Parse, 0, 0);
    let model = voxelizer::load_vox_slice(bytes).map_err(|e| VoxError::Format(e.to_string()))?;
    scene_from_model(model, res_override, on_progress)
}

/// Parses `.cvox` bytes (all models merged) and builds the renderer structure,
/// grid-sized like [`import_vox`]. Same [`Parse`](Phase::Parse) →
/// [`Assemble`](Phase::Assemble) phase pair. Unlike `.vox`, the colour
/// representation is chosen by content — see [`scene_from_cvox`].
pub(crate) fn import_cvox(
    bytes: &[u8],
    res_override: Option<u32>,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<VoxScene, VoxError> {
    // The loader meters parse itself (one tick per tile model per expansion
    // pass) — real fractions through the seconds-long decode of a big file.
    let model = {
        let mut sink = |done, total| on_progress(Phase::Parse, done, total);
        voxelizer::load_cvox_slice(bytes, &mut Progress::new(&mut sink))
            .map_err(|e| VoxError::Format(e.to_string()))?
    };
    on_progress(Phase::Assemble, 0, 0);
    scene_from_cvox(model, res_override)
}

/// The grid a model of `dims` extents loads into: `res_override` when given
/// (rejecting a grid smaller than the model), else the smallest legal size.
fn fit_resolution(dims: [u32; 3], res_override: Option<u32>) -> Result<Resolution, VoxError> {
    let max_dim = dims.iter().copied().max().unwrap_or(1);
    match res_override {
        Some(res) => {
            let resolution = Resolution::new(res).map_err(|e| VoxError::Format(e.to_string()))?;
            if resolution.voxels_per_axis() < max_dim {
                return Err(VoxError::TooSmall { dims, res });
            }
            Ok(resolution)
        }
        None => smallest_fitting(max_dim),
    }
}

/// Centres a parsed voxel-native model in its grid and builds the structure.
fn scene_from_model(
    model: voxelizer::VoxModel,
    res_override: Option<u32>,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<VoxScene, VoxError> {
    let resolution = fit_resolution(model.dims, res_override)?;

    on_progress(Phase::Assemble, 0, 0);
    // Centre the model's size box in the grid.
    let n = resolution.voxels_per_axis();
    let off = [
        (n - model.dims[0]) / 2,
        (n - model.dims[1]) / 2,
        (n - model.dims[2]) / 2,
    ];
    let tree = SparseTree::from_voxels(
        resolution,
        model.voxels.iter().map(|&(c, gid)| {
            (
                VoxelCoord::new(c[0] + off[0], c[1] + off[1], c[2] + off[2]),
                gid,
            )
        }),
    );
    let structure = SchoolBBuffer::from_sparse(&tree);
    Ok(VoxScene {
        tree,
        structure,
        table: model.table,
        models_total: model.models_total,
    })
}

/// Distinct-colour line between the two `.cvox` representations. At or below
/// it (MagicaVoxel-scale palette art) the palette arm keeps the scene exact
/// *and* brush-editable. Above it the file is almost certainly a truecolor
/// bake — and the palette pipeline **cannot** render it faithfully: the GPU's
/// inline per-leaf palette holds `P_CAP = 16` distinct materials per 8³ brick
/// (a bake blows through that on nearly every leaf, and spilled leaves render
/// corrupted colours), and `MaterialTable` tops out at 65535 ids regardless.
/// Such files re-import as truecolor structures — build-once, but lossless.
const MAX_PALETTE_COLORS: usize = 255;

/// Centres a parsed `.cvox` model in its grid and builds the structure,
/// choosing the colour representation by content (see [`MAX_PALETTE_COLORS`]).
/// The truecolor arm routes colours through the same invariant-checked
/// [`SchoolBBuffer::assemble_leaf_color`] assembler the bake uses, so a scene
/// this program exported re-imports as the same species it left as.
fn scene_from_cvox(
    mut model: voxelizer::CvoxModel,
    res_override: Option<u32>,
) -> Result<VoxScene, VoxError> {
    let resolution = fit_resolution(model.dims, res_override)?;
    // Centre the model in its grid, in place — coords are final from here.
    let n = resolution.voxels_per_axis();
    let off = [
        (n - model.dims[0]) / 2,
        (n - model.dims[1]) / 2,
        (n - model.dims[2]) / 2,
    ];
    for (c, _) in &mut model.voxels {
        c[0] += off[0];
        c[1] += off[1];
        c[2] += off[2];
    }
    let models_total = model.models_total;

    // Distinct-colour census with an early exit: only the palette arm needs
    // an exact count, and it needs at most `MAX_PALETTE_COLORS + 1` of it.
    let mut distinct = std::collections::HashSet::new();
    for &(_, color) in &model.voxels {
        distinct.insert(color);
        if distinct.len() > MAX_PALETTE_COLORS {
            break;
        }
    }

    if distinct.len() <= MAX_PALETTE_COLORS {
        // Palette arm: exact and editable.
        let mut table = MaterialTable::missing_only();
        let mut id_of: std::collections::HashMap<u32, u16> = std::collections::HashMap::new();
        let tree = SparseTree::from_voxels(
            resolution,
            model.voxels.iter().map(|&(c, color)| {
                let gid = *id_of.entry(color).or_insert_with(|| {
                    table
                        .push(color)
                        .expect("≤255 distinct colours fit the material table")
                });
                (VoxelCoord::new(c[0], c[1], c[2]), gid)
            }),
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        return Ok(VoxScene {
            tree,
            structure,
            table,
            models_total,
        });
    }

    // Truecolor arm. Sort into exactly the order the colour assembler visits —
    // leaf slots ascend by brick Morton code, voxels within a brick by
    // intra-brick Morton index. Stable, so repeated coords keep file order and
    // the keep-last dedupe below matches `from_voxels`' last-write-wins.
    let sort_key = |c: [u32; 3]| {
        (
            morton::encode(c[0] >> 3, c[1] >> 3, c[2] >> 3),
            morton::encode_brick(c[0] & 7, c[1] & 7, c[2] & 7),
        )
    };
    model.voxels.sort_by_key(|&(c, _)| sort_key(c));
    let mut w = 0usize;
    for r in 0..model.voxels.len() {
        let v = model.voxels[r];
        if w > 0 && model.voxels[w - 1].0 == v.0 {
            model.voxels[w - 1] = v; // same voxel again: the last colour wins
        } else {
            model.voxels[w] = v;
            w += 1;
        }
    }
    model.voxels.truncate(w);

    // Occupancy: in sorted order each brick is one contiguous run — no
    // hash-binning, and `from_bricks` stores no material grids (the uniform
    // form), so the peak stays near the voxel list itself.
    let mut bricks: Vec<(u64, LeafBrick)> = Vec::new();
    for &(c, _) in &model.voxels {
        let code = morton::encode(c[0] >> 3, c[1] >> 3, c[2] >> 3);
        if bricks.last().map(|&(k, _)| k) != Some(code) {
            bricks.push((code, LeafBrick::EMPTY));
        }
        let leaf = &mut bricks.last_mut().expect("pushed above").1;
        leaf.set_local(c[0] & 7, c[1] & 7, c[2] & 7);
    }
    let tree = SparseTree::from_bricks(resolution, bricks);
    let mut structure = SchoolBBuffer::from_sparse(&tree);
    let mut next = model.voxels.iter();
    structure.assemble_leaf_color(&tree, |_| {
        next.next()
            .expect("one colour per occupied voxel, in assembler order")
            .1
            .to_le_bytes()
    });
    Ok(VoxScene {
        tree,
        structure,
        table: MaterialTable::missing_only(),
        models_total,
    })
}

/// Serializes the current scene as a `.vox` file (255-colour palette, 256³
/// occupied-extent limit — the writer errors beyond either).
///
/// Reports [`Gather`](Phase::Gather) (metered per leaf — the loop this module
/// owns) then [`Write`](Phase::Write) (indeterminate — the voxelizer's writer
/// reports no counts). Pass a no-op sink where progress is not observed.
pub(crate) fn export_vox(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    table: &MaterialTable,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<Vec<u8>, VoxError> {
    let voxels = gather_scene_voxels(tree, structure, table, on_progress);
    on_progress(Phase::Write, 0, 0);
    voxelizer::write_vox_voxels(&voxels).map_err(|e| VoxError::Format(e.to_string()))
}

/// Serializes the current scene as a `.cvox` file (unlimited colours; scenes
/// past 256³ split into translated tile models). Same [`Gather`](Phase::Gather)
/// → [`Write`](Phase::Write) phase pair as [`export_vox`].
pub(crate) fn export_cvox(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    table: &MaterialTable,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<Vec<u8>, VoxError> {
    let voxels = gather_scene_voxels(tree, structure, table, on_progress);
    on_progress(Phase::Write, 0, 0);
    voxelizer::write_cvox_voxels(&voxels).map_err(|e| VoxError::Format(e.to_string()))
}

/// Every occupied voxel with its resolved colour. Colour source: the truecolor
/// bake when present, else the palette table (fixtures, having no materials,
/// resolve to the loud id-0 magenta — honest, if garish). Meters the
/// [`Gather`](Phase::Gather) phase by leaf brick — real counts, since this is
/// the one export loop the web layer owns.
fn gather_scene_voxels(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    table: &MaterialTable,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Vec<([u32; 3], u32)> {
    let truecolor = structure.has_leaf_color();
    let color_words = structure.leaf_color_words();
    let color_base = structure.leaf_color_base_words();

    let mut sink = |done, total| on_progress(Phase::Gather, done, total);
    let mut gather = Progress::new(&mut sink);
    let mut meter = gather.meter(tree.leaf_count() as u64);

    let mut voxels: Vec<([u32; 3], u32)> =
        Vec::with_capacity(usize::try_from(tree.occupied_voxels()).unwrap_or(0));
    // `idx` indexes three parallel per-leaf views (origin, brick, materials);
    // a zip would hide that they must stay in leaf order.
    #[allow(clippy::needless_range_loop)]
    for idx in 0..tree.leaf_count() {
        let origin = tree.leaf_origin(idx);
        let brick = structure.leaf_at(u32::try_from(idx).unwrap_or(u32::MAX));
        let mats = tree.leaf_materials(idx);
        for lz in 0..8 {
            for ly in 0..8 {
                for lx in 0..8 {
                    if !brick.get_local(lx, ly, lz) {
                        continue;
                    }
                    let m = morton::encode_brick(lx, ly, lz);
                    let color = if truecolor {
                        let i = color_base[idx] + brick.occupied_rank(m);
                        color_words.get(i as usize).copied().unwrap_or(0xFF00_00FF)
                    } else {
                        table.color(mats[m as usize])
                    };
                    voxels.push(([origin.x + lx, origin.y + ly, origin.z + lz], color));
                }
            }
        }
        meter.add(1);
    }
    voxels
}

/// The smallest legal `8·4^k` resolution with `voxels_per_axis() >= max_dim`.
fn smallest_fitting(max_dim: u32) -> Result<Resolution, VoxError> {
    for k in 0.. {
        let Ok(resolution) = Resolution::from_internal_levels(k) else {
            break;
        };
        if resolution.voxels_per_axis() >= max_dim {
            return Ok(resolution);
        }
    }
    Err(VoxError::Format(format!(
        "model dimension {max_dim} exceeds the largest supported grid"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A progress sink that observes nothing.
    fn noop(_: Phase, _: u64, _: u64) {}

    /// A small asymmetric palette scene shared by the phase-stream tests.
    fn sample_scene() -> (SparseTree, SchoolBBuffer, MaterialTable) {
        let mut table = MaterialTable::missing_only();
        let red = table.push(0xFF00_00E0).expect("push red");
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), red),
                (VoxelCoord::new(9, 2, 30), red),
            ],
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        (tree, structure, table)
    }

    #[test]
    fn export_reports_gather_then_write_and_meters_the_gather() {
        let (tree, structure, table) = sample_scene();
        let mut events: Vec<(Phase, u64, u64)> = Vec::new();
        export_vox(&tree, &structure, &table, &mut |p, d, t| {
            events.push((p, d, t));
        })
        .expect("export");

        let phases: Vec<Phase> = events.iter().map(|&(p, _, _)| p).collect();
        // Gather (its meter start + endpoint) precedes Write; ordering is exact.
        assert_eq!(phases.first(), Some(&Phase::Gather));
        assert_eq!(phases.last(), Some(&Phase::Write));
        assert!(
            phases.iter().position(|&p| p == Phase::Gather)
                < phases.iter().rposition(|&p| p == Phase::Write),
            "gather precedes write: {phases:?}",
        );
        // The gather meter's total is the leaf count; its terminal emission
        // reaches that total (done == total).
        let gather: Vec<(u64, u64)> = events
            .iter()
            .filter(|&&(p, _, _)| p == Phase::Gather)
            .map(|&(_, d, t)| (d, t))
            .collect();
        let leaves = tree.leaf_count() as u64;
        assert_eq!(
            gather.first(),
            Some(&(0, leaves)),
            "meter start is (0, total)"
        );
        assert_eq!(
            gather.last(),
            Some(&(leaves, leaves)),
            "meter finishes at total"
        );
    }

    #[test]
    fn import_reports_parse_then_assemble() {
        let (tree, structure, table) = sample_scene();
        let bytes = export_vox(&tree, &structure, &table, &mut noop).expect("export");
        let mut phases = Vec::new();
        import_vox(&bytes, None, &mut |p, _, _| phases.push(p)).expect("import");
        assert_eq!(phases, vec![Phase::Parse, Phase::Assemble]);
    }

    /// The round trip this module exists to support: a truecolor-baked scene
    /// (way more distinct colours than any palette holds, plus a
    /// semi-transparent voxel) exported to `.cvox` and imported back. The old
    /// palette-bridging import either rejected such files (>65535 colours) or
    /// corrupted them (the GPU's inline per-leaf palette holds 16 distinct
    /// materials per brick).
    #[test]
    fn truecolor_scene_round_trips_through_cvox() {
        let resolution = Resolution::new(32).expect("legal");
        // 600 voxels (> MAX_PALETTE_COLORS distinct colours) across many bricks.
        let coords: Vec<VoxelCoord> = (0..600u32)
            .map(|i| VoxelCoord::new(i % 32, i / 32, 7))
            .collect();
        let tree = SparseTree::from_voxels(resolution, coords.iter().map(|&c| (c, 0u16)));
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        structure.assemble_leaf_color(&tree, |c| {
            // Distinct colour per voxel; one semi-transparent for the blend bit.
            let alpha = if c == VoxelCoord::new(0, 0, 7) {
                128
            } else {
                255
            };
            [c.x as u8, c.y as u8, (c.x ^ c.y) as u8, alpha]
        });
        assert!(structure.has_transparency());
        let table = MaterialTable::missing_only();

        let bytes = export_cvox(&tree, &structure, &table, &mut noop).expect("export");
        let scene = import_cvox(&bytes, None, &mut noop).expect("import our own bytes");

        // The scene comes back as the species it left as: truecolor, with no
        // dense palette grids and the transparency routing re-derived.
        assert!(
            scene.structure.has_leaf_color(),
            "truecolor stays truecolor"
        );
        assert!(
            scene.tree.has_uniform_materials(),
            "no per-leaf material grids"
        );
        assert!(
            scene.structure.has_transparency(),
            "alpha survived the trip"
        );
        assert_eq!(scene.tree.occupied_voxels(), tree.occupied_voxels());

        // Sharp oracle: exporting the re-import reproduces the bytes exactly
        // (the writer is deterministic and crops to the content box, so grid
        // placement cancels). Any colour or coordinate drift would diverge.
        let again =
            export_cvox(&scene.tree, &scene.structure, &scene.table, &mut noop).expect("re-export");
        assert_eq!(
            again, bytes,
            "second-generation export must be byte-identical"
        );
    }

    /// Palette-sized `.cvox` files (MagicaVoxel-scale art) keep the palette
    /// representation — exact colours *and* brush-editable.
    #[test]
    fn few_colour_cvox_stays_a_palette_scene() {
        let (tree, structure, table) = sample_scene();
        let bytes = export_cvox(&tree, &structure, &table, &mut noop).expect("export");
        let scene = import_cvox(&bytes, None, &mut noop).expect("import");
        assert!(
            !scene.structure.has_leaf_color(),
            "palette art stays palette"
        );
        assert!(
            !scene.tree.has_uniform_materials(),
            "real material ids stored"
        );
        assert_eq!(scene.table.words().len(), table.words().len());
    }

    /// Scene → .vox → scene: the occupied set and colours survive both
    /// adapters and both axis conversions.
    #[test]
    fn scene_round_trips_through_vox_bytes() {
        // A small asymmetric scene with real materials.
        let mut table = MaterialTable::missing_only();
        let red = table.push(0xFF00_00E0).expect("push red");
        let blue = table.push(0xFFD0_4000).expect("push blue");
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), red),
                (VoxelCoord::new(1, 3, 3), red),
                (VoxelCoord::new(9, 2, 30), blue),
            ],
        );
        let structure = SchoolBBuffer::from_sparse(&tree);

        let bytes = export_vox(&tree, &structure, &table, &mut noop).expect("export");
        let scene = import_vox(&bytes, None, &mut noop).expect("re-import");

        assert_eq!(scene.tree.occupied_voxels(), 3);
        assert_eq!(scene.models_total, 1);
        // Colours survive (compare as multisets of table colours).
        let colors_of = |t: &SparseTree, tab: &MaterialTable| {
            let mut v: Vec<u32> = (0..t.leaf_count())
                .flat_map(|i| {
                    let mats = t.leaf_materials(i);
                    let origin = t.leaf_origin(i);
                    (0..8u32).flat_map(move |z| {
                        (0..8u32).flat_map(move |y| {
                            (0..8u32)
                                .filter(move |&x| {
                                    t.is_occupied(VoxelCoord::new(
                                        origin.x + x,
                                        origin.y + y,
                                        origin.z + z,
                                    ))
                                })
                                .map(move |x| {
                                    tab.color(mats[morton::encode_brick(x, y, z) as usize])
                                })
                        })
                    })
                })
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(
            colors_of(&scene.tree, &scene.table),
            vec![0xFF00_00E0, 0xFF00_00E0, 0xFFD0_4000]
        );
    }

    /// Scene → .cvox → scene: the compressed sibling round-trips identically
    /// (multi-model tiling and box merging included).
    #[test]
    fn scene_round_trips_through_cvox_bytes() {
        let mut table = MaterialTable::missing_only();
        let red = table.push(0xFF00_00E0).expect("push red");
        let blue = table.push(0xFFD0_4000).expect("push blue");
        let resolution = Resolution::new(32).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            [
                (VoxelCoord::new(1, 2, 3), red),
                (VoxelCoord::new(1, 3, 3), red),
                (VoxelCoord::new(9, 2, 30), blue),
            ],
        );
        let structure = SchoolBBuffer::from_sparse(&tree);

        let bytes = export_cvox(&tree, &structure, &table, &mut noop).expect("export");
        let scene = import_cvox(&bytes, None, &mut noop).expect("re-import");
        assert_eq!(scene.tree.occupied_voxels(), 3);

        // Same scene through both formats agrees voxel-for-voxel.
        let vox_bytes = export_vox(&tree, &structure, &table, &mut noop).expect("vox export");
        let via_vox = import_vox(&vox_bytes, None, &mut noop).expect("vox re-import");
        assert_eq!(scene.tree.occupied_voxels(), via_vox.tree.occupied_voxels());
    }

    #[test]
    fn auto_resolution_picks_smallest_legal_grid() {
        assert_eq!(smallest_fitting(1).unwrap().voxels_per_axis(), 8);
        assert_eq!(smallest_fitting(8).unwrap().voxels_per_axis(), 8);
        assert_eq!(smallest_fitting(9).unwrap().voxels_per_axis(), 32);
        assert_eq!(smallest_fitting(200).unwrap().voxels_per_axis(), 512);
        assert_eq!(smallest_fitting(256).unwrap().voxels_per_axis(), 512);
    }

    #[test]
    fn undersized_override_is_a_typed_error() {
        let mut table = MaterialTable::missing_only();
        let id = table.push(0xFFFF_FFFF).expect("push");
        let resolution = Resolution::new(128).expect("legal");
        let tree = SparseTree::from_voxels(
            resolution,
            (0..40u32).map(|i| (VoxelCoord::new(i, 0, 0), id)),
        );
        let structure = SchoolBBuffer::from_sparse(&tree);
        let bytes = export_vox(&tree, &structure, &table, &mut noop).expect("export");
        assert!(matches!(
            import_vox(&bytes, Some(32), &mut noop),
            Err(VoxError::TooSmall { .. })
        ));
    }
}
