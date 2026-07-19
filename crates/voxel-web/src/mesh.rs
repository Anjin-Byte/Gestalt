//! Mesh-bytes → voxel structure for the web engine's control plane.
//!
//! The bytes-based, async mirror of the viewer's `build_from_mesh`: parse with
//! the voxelizer's `_slice` loaders, voxelize on the GPU via the sparse compact
//! path (async end-to-end, so it runs on WebGPU without blocking), then
//! optionally MASK-cutout and bake per-voxel truecolor. Progress prints are
//! omitted — the caller reports through `SceneInfo`/errors instead.

use glam::Mat4;
use voxel_core::{MaterialTable, Progress, Resolution, SchoolBBuffer, SparseTree};
use voxelizer::{GpuVoxelizer, GpuVoxelizerConfig, MeshInput, VoxelGrid, VoxelizeOpts};

use crate::phases::Phase;

/// The mesh container format of an import payload.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MeshKind {
    /// glTF binary (`.glb`) or JSON with embedded buffers (`.gltf`).
    Gltf,
    /// Wavefront OBJ — geometry only from bytes (no `.mtl`/texture resolution).
    Obj,
    /// STL, binary or ASCII.
    Stl,
}

/// Knobs for a mesh import, already validated/defaulted by the boundary layer.
pub(crate) struct MeshBuildOptions {
    /// Applied before the grid fit (re-orients transform-less formats).
    pub(crate) rotation: Mat4,
    /// Voxels of margin around the mesh's bounding box when fitting the grid.
    pub(crate) padding: f32,
    /// Bake per-voxel truecolor when the mesh is textured (glTF).
    pub(crate) truecolor: bool,
    /// Run the colour bake on the GPU (falls back to the CPU oracle on an
    /// unsupported device); `false` = the A/B comparison path.
    pub(crate) gpu_bake: bool,
}

/// Why a mesh import failed, in shell-reportable terms.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MeshBuildError {
    /// The bytes did not parse as the declared format.
    #[error("mesh load: {0}")]
    Load(String),
    /// The GPU voxelization pipeline rejected or failed the job.
    #[error("voxelize: {0}")]
    Voxelize(String),
    /// Truecolor was requested but the scene is over the colour ceiling.
    #[error(
        "truecolor scene has {voxels} occupied voxels, over the {max}-voxel ceiling; lower the resolution"
    )]
    TruecolorCeiling {
        /// Occupied voxels produced by the compact pass.
        voxels: usize,
        /// The renderer's colour-storage ceiling.
        max: usize,
    },
}

/// The built scene: the live tree, its GPU buffer (with truecolor baked in when
/// requested and textured), and the material table for the palette arm.
pub(crate) struct BuiltMesh {
    pub(crate) tree: SparseTree,
    pub(crate) structure: SchoolBBuffer,
    pub(crate) table: MaterialTable,
}

/// Parses `bytes` per `kind`, voxelizes into a `resolution`³ grid on the given
/// device, and builds the renderer structure. Async end-to-end: every GPU
/// readback awaits, so this runs on both native (pollster) and WebGPU.
///
/// Takes `bytes` by value and frees each intermediate (file bytes, parsed
/// mesh + textures, packed texels, compact voxel list) as soon as the last
/// phase that reads it finishes: on wasm32 the heap's high-water mark is the
/// *sum* of what is simultaneously live, and `memory.grow` never shrinks.
#[allow(clippy::too_many_lines)] // a linear phase pipeline; splitting it would obscure the free-as-you-go flow
pub(crate) async fn build_from_mesh_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resolution: Resolution,
    bytes: Vec<u8>,
    kind: MeshKind,
    opts: &MeshBuildOptions,
    on_progress: &mut impl FnMut(Phase, u64, u64),
) -> Result<BuiltMesh, MeshBuildError> {
    // Currying device: each phase gets the one shared Meter machinery with its
    // label baked into the sink — phases stay labels on one stream.
    macro_rules! phase_sink {
        ($phase:expr) => {
            |done, total| on_progress($phase, done, total)
        };
    }

    on_progress(Phase::Parse, 0, 0); // indeterminate: no unit of work
    let mut mesh = load_slice(&bytes, kind)?;
    drop(bytes); // the file copy is parsed; nothing later re-reads it
    // Re-orient transform-less formats before measuring the bounding box.
    if opts.rotation != Mat4::IDENTITY {
        mesh.transform(opts.rotation);
    }
    // Truecolor only: drop toon-outline inverted hulls before the grid fit, or
    // the surface voxelizer coats the model in a black shell (see the viewer's
    // build_from_mesh for the full rationale).
    if opts.truecolor {
        let _ = mesh.drop_outline_triangles();
    }

    let grid = VoxelGrid::fit_mesh(resolution, &mesh, opts.padding);
    let vopts = VoxelizeOpts {
        epsilon: 1e-4,
        // The compact pass resolves owner→material per occupied voxel.
        store_owner: true,
        store_color: false,
    };
    let vox = GpuVoxelizer::from_device(device, queue, GpuVoxelizerConfig::default())
        .await
        .map_err(|e| MeshBuildError::Voxelize(e.to_string()))?;

    let (table, packed) = voxelizer::material_table_for_sparse(&mesh)
        .map_err(|e| MeshBuildError::Voxelize(e.to_string()))?;
    let baking = opts.truecolor && mesh.appearance.is_some();
    // The mesh (with its decoded textures — usually the parse footprint's bulk)
    // and the packed texel words travel together from here, freed at the
    // earliest phase boundary that no longer reads them.
    let mut parse = Some((mesh, packed));

    // The two GPU stages carry separate labels (each is a real wait at high
    // resolutions), which is why this calls the un-fused pair rather than
    // `compact_surface_sparse`.
    let (mesh, packed) = parse.as_ref().expect("parse products live through compact");
    let chunks = vox
        .voxelize_surface_sparse_chunked(
            mesh,
            &grid,
            &vopts,
            0,
            &mut Progress::new(&mut phase_sink!(Phase::Voxelize)),
        )
        .await
        .map_err(|e| MeshBuildError::Voxelize(e.to_string()))?;
    let voxels = vox
        .compact_chunks(
            &chunks,
            packed,
            [0, 0, 0],
            &mut Progress::new(&mut phase_sink!(Phase::Compact)),
        )
        .await
        .map_err(|e| MeshBuildError::Voxelize(e.to_string()))?;
    drop(chunks);

    // MASK alpha-cutout (truecolor only) must run before the tree build —
    // clearing occupancy afterwards would corrupt `occupied_rank`.
    let voxels = if opts.truecolor {
        let (mesh, packed) = parse.as_ref().expect("parse products live through cutout");
        voxelizer::cull_mask_cutout(
            &voxels,
            mesh,
            &grid,
            vopts.epsilon,
            Some(packed),
            &mut Progress::new(&mut phase_sink!(Phase::Cutout)),
        )
    } else {
        voxels
    };
    if !baking {
        parse = None; // textures + texel words are dead weight past the cutout
    }
    // The truecolor ceiling depends only on the compact count; check before
    // assembling so an over-ceiling build fails without the tree allocation.
    if baking && voxels.len() > voxel_gpu::MAX_TRUECOLOR_VOXELS {
        return Err(MeshBuildError::TruecolorCeiling {
            voxels: voxels.len(),
            max: voxel_gpu::MAX_TRUECOLOR_VOXELS,
        });
    }

    let (tree, _dropped) = voxelizer::tree_from_compact(
        resolution,
        &voxels,
        &mut Progress::new(&mut phase_sink!(Phase::Assemble)),
    );
    drop(voxels); // ~16 B per occupied voxel; the tree carries everything now
    let mut structure = SchoolBBuffer::from_sparse(&tree);

    // Optional per-voxel truecolor bake: textured meshes only (an untextured
    // mesh keeps the palette path).
    if baking {
        let (mesh, packed) = parse
            .as_ref()
            .expect("parse products live through the bake");
        bake_colors(
            &vox,
            &tree,
            &mut structure,
            mesh,
            &grid,
            vopts.epsilon,
            packed,
            opts.gpu_bake,
            &mut |done, total| on_progress(Phase::ColorBake, done, total),
        )
        .await;
    }

    Ok(BuiltMesh {
        tree,
        structure,
        table,
    })
}

/// The colour-bake arm: GPU over the packed inputs when requested (colours
/// install through the invariant-checked assembler, which also derives the
/// transparency bits), falling back to the CPU oracle on a device rejection —
/// never on silence.
#[allow(clippy::too_many_arguments)] // the bake's full context, one call site
async fn bake_colors(
    vox: &GpuVoxelizer,
    tree: &SparseTree,
    structure: &mut SchoolBBuffer,
    mesh: &MeshInput,
    grid: &VoxelGrid,
    epsilon: f32,
    packed: &[u32],
    gpu_bake: bool,
    sink: &mut dyn FnMut(u64, u64),
) {
    if gpu_bake {
        let inputs =
            voxelizer::pack_bake_inputs(tree, structure, mesh, grid, epsilon, Some(packed));
        if let Ok(colors) = vox
            .bake_leaf_colors_gpu(&inputs, &mut Progress::new(sink))
            .await
        {
            let mut next = colors.into_iter();
            structure.assemble_leaf_color(tree, |_| {
                next.next()
                    .expect("gpu bake returns one colour per occupied voxel")
                    .to_le_bytes()
            });
            return;
        }
    }
    voxelizer::bake_leaf_colors(
        structure,
        tree,
        mesh,
        grid,
        epsilon,
        Some(packed),
        &mut Progress::new(sink),
    );
}

/// Dispatches to the format's `_slice` loader.
fn load_slice(bytes: &[u8], kind: MeshKind) -> Result<MeshInput, MeshBuildError> {
    let loaded = match kind {
        MeshKind::Gltf => voxelizer::load_gltf_slice(bytes),
        MeshKind::Obj => voxelizer::load_obj_slice(bytes),
        MeshKind::Stl => voxelizer::load_stl_slice(bytes),
    };
    loaded.map_err(|e| MeshBuildError::Load(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Native smoke of the exact flow the browser runs: bytes → loader → GPU
    /// voxelize → truecolor bake, driven by pollster on the native adapter.
    /// The wasm target differs only in who drives the event loop. Skips
    /// (passes trivially) without a GPU or the reference model, mirroring the
    /// workspace's differential-test convention.
    #[test]
    fn glb_bytes_voxelize_and_bake_truecolor() {
        let Ok(ctx) = voxel_gpu::GpuContext::try_new() else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../models/gltf/LittlestTokyo.glb"
        );
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skipping: reference model not present ({path})");
            return;
        };

        let resolution = Resolution::new(128).expect("128 = 8·4^2 is legal");
        let opts = MeshBuildOptions {
            rotation: Mat4::IDENTITY,
            padding: 2.0,
            truecolor: true,
            gpu_bake: true,
        };
        let mut events: Vec<(Phase, u64, u64)> = Vec::new();
        let built = pollster::block_on(build_from_mesh_bytes(
            &ctx.device,
            &ctx.queue,
            resolution,
            bytes,
            MeshKind::Gltf,
            &opts,
            &mut |phase, done, total| events.push((phase, done, total)),
        ))
        .expect("voxelize LittlestTokyo.glb at 128³");

        // The progress stream is the build's observable contract: phases in
        // order with no interleaving, `done` monotone and bounded by a stable
        // `total` within each phase, and every determinate phase finishing at
        // its total.
        let mut phase_order: Vec<Phase> = Vec::new();
        for &(phase, _, _) in &events {
            if phase_order.last() != Some(&phase) {
                assert!(
                    !phase_order.contains(&phase),
                    "phase {phase:?} re-entered: phases must not interleave"
                );
                phase_order.push(phase);
            }
        }
        assert_eq!(
            phase_order,
            vec![
                Phase::Parse,
                Phase::Voxelize,
                Phase::Compact,
                Phase::Cutout,
                Phase::Assemble,
                Phase::ColorBake,
            ]
        );
        for phase in &phase_order {
            let run: Vec<(u64, u64)> = events
                .iter()
                .filter(|(p, _, _)| p == phase)
                .map(|&(_, d, t)| (d, t))
                .collect();
            let total = run[0].1;
            assert!(run.iter().all(|&(d, t)| t == total && d <= total));
            assert!(
                run.windows(2).all(|w| w[0].0 <= w[1].0),
                "{phase:?} not monotone"
            );
            if total > 0 {
                assert_eq!(
                    run.last().expect("non-empty").0,
                    total,
                    "{phase:?} unfinished"
                );
            }
        }
        assert!(built.tree.leaf_count() > 0, "mesh produced no leaves");
        assert!(built.tree.occupied_voxels() > 0, "mesh produced no voxels");
        assert!(
            built.structure.has_leaf_color(),
            "textured GLB with truecolor on should bake leaf colors"
        );
        assert_eq!(built.structure.node_count(), built.tree.node_count());
        let _ = built.table;
    }
}
