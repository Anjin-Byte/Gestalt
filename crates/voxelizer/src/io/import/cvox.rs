//! `.cvox` input adapter — `JelleBouma`'s losslessly compressed voxel format
//! (<https://github.com/JelleBouma/cvox>, spec `cvox v1.txt`).
//!
//! Voxel-native like [`super::vox`], but compressed: same-coloured voxels are
//! stored as inclusive-bound cubes (`CMAP`+`CUBE`) with loose singles in
//! (`VMAP`+`XYZ `), and colours are full RGBA (no 255-colour palette limit).
//! Multi-model files carry plain integer translations — no scene graph — so
//! **all models merge** into one scene here, unlike the `.vox` adapter.
//!
//! Same axis convention as `.vox` (the spec marks `size z` as the gravity
//! direction): right-handed Z-up, converted to this renderer's Y-up by the
//! proper rotation `(x, y, z)_engine = (x, z, maxY − y)_global`.
//!
//! One deliberate spec deviation, matching the reference implementation: the
//! spec's §7 says `VMAP` colours are ARGB-ordered, but the reference reader
//! and writer use RGBA for **both** maps (`ColourMap` round-trips `ToRgba()`),
//! and interop with its files is the point — RGBA everywhere.
//!
//! Colours come back **raw** (a [`CvoxModel`], not a palette): the format has
//! no colour ceiling and a truecolor export carries millions of distinct
//! colours — far past any palette representation. Expansion runs in two
//! streaming passes (bounds, then a fill straight into final engine coords)
//! so a tens-of-millions-of-voxels file peaks at one 16-byte entry per voxel,
//! not the 32-byte global-space intermediate a single-pass normalize needs —
//! on wasm32 that difference is the line between loading and an OOM abort.

use voxel_core::Progress;

use super::CvoxModel;
use crate::error::VoxelizerError;

/// Guard against decompression bombs: a `CUBE` chunk is 6 bytes per cube but
/// can declare a 256³ solid each (16.7M voxels). Cap the expanded total at one
/// full 2048-grid brick layer's worth — far beyond any legitimate model this
/// renderer can hold, small enough to fail fast on hostile files.
const MAX_EXPANDED_VOXELS: u64 = 64 << 20;

/// A parsed chunk header: id + content slice.
struct Chunk<'a> {
    id: [u8; 4],
    content: &'a [u8],
}

/// Reads the chunk at `bytes[*at..]`, advancing `at` past it.
fn read_chunk<'a>(bytes: &'a [u8], at: &mut usize) -> Result<Chunk<'a>, VoxelizerError> {
    let header = bytes
        .get(*at..*at + 8)
        .ok_or_else(|| VoxelizerError::MeshLoad("cvox: truncated chunk header".to_string()))?;
    let id = [header[0], header[1], header[2], header[3]];
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let content = bytes
        .get(*at + 8..*at + 8 + len)
        .ok_or_else(|| VoxelizerError::MeshLoad("cvox: truncated chunk content".to_string()))?;
    *at += 8 + len;
    Ok(Chunk { id, content })
}

/// One colour-map entry: RGBA8-LE colour + how many following geometry entries
/// use it (zero-count entries are legal "unused palette" storage).
fn read_map(content: &[u8]) -> Result<Vec<(u32, u32)>, VoxelizerError> {
    if !content.len().is_multiple_of(7) {
        return Err(VoxelizerError::MeshLoad(
            "cvox: colour map length not divisible by 7".to_string(),
        ));
    }
    Ok(content
        .chunks_exact(7)
        .map(|e| {
            let color = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let count = u32::from_le_bytes([e[4], e[5], e[6], 0]);
            (color, count)
        })
        .collect())
}

/// The in-flight state of one model between its `SIZE` chunk and the next.
#[derive(Default)]
struct PendingModel {
    translation: [i64; 3],
    cmap: Vec<(u32, u32)>,
    cubes: Vec<([u8; 3], [u8; 3])>,
    vmap: Vec<(u32, u32)>,
    voxels: Vec<[u8; 3]>,
}

/// Fills one geometry/map chunk into the pending model.
fn read_model_chunk(model: &mut PendingModel, chunk: &Chunk<'_>) -> Result<(), VoxelizerError> {
    match &chunk.id {
        b"CMAP" => model.cmap = read_map(chunk.content)?,
        b"VMAP" => model.vmap = read_map(chunk.content)?,
        b"CUBE" => {
            if !chunk.content.len().is_multiple_of(6) {
                return Err(VoxelizerError::MeshLoad(
                    "cvox: CUBE length not divisible by 6".to_string(),
                ));
            }
            model.cubes = chunk
                .content
                .chunks_exact(6)
                .map(|c| ([c[0], c[1], c[2]], [c[3], c[4], c[5]]))
                .collect();
        }
        _ => {
            if !chunk.content.len().is_multiple_of(3) {
                return Err(VoxelizerError::MeshLoad(
                    "cvox: XYZ length not divisible by 3".to_string(),
                ));
            }
            model.voxels = chunk
                .content
                .chunks_exact(3)
                .map(|v| [v[0], v[1], v[2]])
                .collect();
        }
    }
    Ok(())
}

/// Parses `.cvox` bytes, merging every model (translations applied) into one
/// engine-space [`CvoxModel`] with raw per-voxel colours.
///
/// Meters `progress` over `2 × models` — one tick per model per expansion
/// pass — so a large tiled file (this renderer's own exports tile at 256³:
/// hundreds of models for a 2048-grid scene) reports real fractions through
/// what is otherwise a seconds-long silent decode. Pass [`Progress::none`]
/// where progress is not observed.
///
/// # Errors
/// [`VoxelizerError::MeshLoad`] on malformed bytes, colour-map/geometry count
/// mismatches, or an expanded voxel count over the decompression-bomb cap.
pub fn load_cvox_slice(bytes: &[u8], progress: &mut Progress) -> Result<CvoxModel, VoxelizerError> {
    let mut at = 0usize;
    let first = read_chunk(bytes, &mut at)?;
    if &first.id != b"CVOX" {
        return Err(VoxelizerError::MeshLoad(
            "cvox: not a cvox file (missing CVOX chunk)".to_string(),
        ));
    }
    let version = first
        .content
        .get(..4)
        .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .ok_or_else(|| VoxelizerError::MeshLoad("cvox: truncated version".to_string()))?;
    if version != 1 {
        return Err(VoxelizerError::MeshLoad(format!(
            "cvox: unsupported version {version} (expected 1)"
        )));
    }

    // Walk chunks, flushing the pending model when the next SIZE (or EOF)
    // arrives. Unknown chunk ids are skipped per the spec.
    let mut models: Vec<PendingModel> = Vec::new();
    let mut current: Option<PendingModel> = None;
    while at < bytes.len() {
        let chunk = read_chunk(bytes, &mut at)?;
        match &chunk.id {
            b"SIZE" => {
                if let Some(done) = current.take() {
                    models.push(done);
                }
                if chunk.content.len() < 15 {
                    return Err(VoxelizerError::MeshLoad(
                        "cvox: SIZE chunk shorter than 15 bytes".to_string(),
                    ));
                }
                let t = |o: usize| {
                    i64::from(u32::from_le_bytes([
                        chunk.content[o],
                        chunk.content[o + 1],
                        chunk.content[o + 2],
                        chunk.content[o + 3],
                    ]))
                };
                current = Some(PendingModel {
                    // The declared size bytes are ignored: extents derive from
                    // content (robust to the reference writer's 256→0 byte
                    // wrap), and placement comes from the translation.
                    translation: [t(3), t(7), t(11)],
                    ..PendingModel::default()
                });
            }
            b"CMAP" | b"VMAP" | b"CUBE" | b"XYZ " => {
                let model = current.as_mut().ok_or_else(|| {
                    VoxelizerError::MeshLoad(format!(
                        "cvox: {} chunk before any SIZE chunk",
                        String::from_utf8_lossy(&chunk.id)
                    ))
                })?;
                read_model_chunk(model, &chunk)?;
            }
            _ => {} // unknown chunk: skip per spec
        }
    }
    if let Some(done) = current.take() {
        models.push(done);
    }
    if models.is_empty() {
        return Err(VoxelizerError::MeshLoad(
            "cvox: file contains no models".to_string(),
        ));
    }
    assemble_model(&models, progress)
}

/// Streams the parsed models through two bounded-memory expansion passes into
/// normalized engine-space voxels (see the module docs for the rationale),
/// ticking `progress` once per model per pass.
fn assemble_model(
    models: &[PendingModel],
    progress: &mut Progress,
) -> Result<CvoxModel, VoxelizerError> {
    let models_total = models.len();
    let mut meter = progress.meter(2 * models_total as u64);

    // Pass 1 — stream the expansion to find the global bounds and total count
    // (the decompression-bomb cap trips here). Nothing is stored.
    let mut bounds: Option<([i64; 3], [i64; 3])> = None;
    let mut count = 0usize;
    let mut expanded: u64 = 0;
    for model in models {
        expand_geometry(model, &mut expanded, &mut |c, _| {
            let (lo, hi) = bounds.get_or_insert((c, c));
            for a in 0..3 {
                lo[a] = lo[a].min(c[a]);
                hi[a] = hi[a].max(c[a]);
            }
            count += 1;
        })?;
        meter.add(1);
    }
    let Some((lo, hi)) = bounds else {
        return Err(VoxelizerError::MeshLoad(
            "cvox: file contains no voxels".to_string(),
        ));
    };
    let max_y = hi[1] - lo[1]; // global Y extent − 1, the flip pivot

    // Pass 2 — re-expand straight into normalized, Z-up → Y-up rotated engine
    // coords (the cap already passed; the second counter just satisfies the
    // shared signature).
    let mut voxels: Vec<([u32; 3], u32)> = Vec::with_capacity(count);
    let mut expanded2: u64 = 0;
    for model in models {
        expand_geometry(model, &mut expanded2, &mut |c, color| {
            let (gx, gy, gz) = (c[0] - lo[0], c[1] - lo[1], c[2] - lo[2]);
            let coord = [
                u32::try_from(gx).expect("normalized to >= 0"),
                u32::try_from(gz).expect("normalized to >= 0"),
                u32::try_from(max_y - gy).expect("flip pivot is the max"),
            ];
            voxels.push((coord, color));
        })?;
        meter.add(1);
    }

    let dims = [
        u32::try_from(hi[0] - lo[0] + 1).expect("cropped extent"),
        u32::try_from(hi[2] - lo[2] + 1).expect("cropped extent"),
        u32::try_from(hi[1] - lo[1] + 1).expect("cropped extent"),
    ];
    Ok(CvoxModel {
        voxels,
        dims,
        models_total,
    })
}

/// Streams one model's cubes + singles through `visit` as
/// `(global Z-up coord, colour)` at the model's translation, enforcing the
/// colour-map/geometry pairing and the expansion cap. A visitor rather than a
/// buffer so callers can make bounded-memory passes (see [`load_cvox_slice`]).
fn expand_geometry(
    model: &PendingModel,
    expanded: &mut u64,
    visit: &mut impl FnMut([i64; 3], u32),
) -> Result<(), VoxelizerError> {
    let t = model.translation;

    // Cubes: the CMAP's run-lengths assign colours to CUBE entries in order.
    let mut cube_iter = model.cubes.iter();
    let mut cubes_covered = 0usize;
    for &(color, count) in &model.cmap {
        for _ in 0..count {
            let &(lo, hi) = cube_iter.next().ok_or_else(|| {
                VoxelizerError::MeshLoad(
                    "cvox: CMAP counts exceed the CUBE entry count".to_string(),
                )
            })?;
            cubes_covered += 1;
            for a in 0..3 {
                if lo[a] > hi[a] {
                    return Err(VoxelizerError::MeshLoad(
                        "cvox: cube low coordinate above high".to_string(),
                    ));
                }
            }
            let volume = (u64::from(hi[0] - lo[0]) + 1)
                * (u64::from(hi[1] - lo[1]) + 1)
                * (u64::from(hi[2] - lo[2]) + 1);
            *expanded += volume;
            if *expanded > MAX_EXPANDED_VOXELS {
                return Err(VoxelizerError::MeshLoad(format!(
                    "cvox: expanded voxel count exceeds the {MAX_EXPANDED_VOXELS} cap"
                )));
            }
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        visit(
                            [
                                t[0] + i64::from(x),
                                t[1] + i64::from(y),
                                t[2] + i64::from(z),
                            ],
                            color,
                        );
                    }
                }
            }
        }
    }
    if cubes_covered != model.cubes.len() {
        return Err(VoxelizerError::MeshLoad(
            "cvox: CUBE entries not covered by CMAP counts".to_string(),
        ));
    }

    // Singles: VMAP run-lengths over XYZ entries, same pairing rule.
    let mut voxel_iter = model.voxels.iter();
    let mut voxels_covered = 0usize;
    for &(color, count) in &model.vmap {
        for _ in 0..count {
            let &v = voxel_iter.next().ok_or_else(|| {
                VoxelizerError::MeshLoad("cvox: VMAP counts exceed the XYZ entry count".to_string())
            })?;
            voxels_covered += 1;
            *expanded += 1;
            if *expanded > MAX_EXPANDED_VOXELS {
                return Err(VoxelizerError::MeshLoad(format!(
                    "cvox: expanded voxel count exceeds the {MAX_EXPANDED_VOXELS} cap"
                )));
            }
            visit(
                [
                    t[0] + i64::from(v[0]),
                    t[1] + i64::from(v[1]),
                    t[2] + i64::from(v[2]),
                ],
                color,
            );
        }
    }
    if voxels_covered != model.voxels.len() {
        return Err(VoxelizerError::MeshLoad(
            "cvox: XYZ entries not covered by VMAP counts".to_string(),
        ));
    }
    Ok(())
}
