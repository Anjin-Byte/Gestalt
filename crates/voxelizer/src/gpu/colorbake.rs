//! GPU colour-bake, tranche 2: the **packed input layout** and its CPU
//! reference reader (`docs/design/web-frontend-api.md` §9).
//!
//! Everything the future WGSL kernel reads is flattened here into
//! GPU-uploadable buffers: one texel word buffer for every texture (no
//! samplers — WebGPU has no bindless texture arrays, and the pinned bake does
//! *manual* bilinear anyway), per-triangle candidate records in grid space,
//! CSR candidate ranges per leaf, and the leaf occupancy/material/base data.
//!
//! [`reference_bake`] walks those packed buffers with the **same generic
//! oracle code** the production bake runs ([`TexelSource`] keeps the sampling
//! algorithm single-sourced), which pins the packing itself: the differential
//! test asserts packed-reference output ≡ `bake_leaf_colors` word-for-word.
//! The WGSL kernel (tranche 3) transcribes exactly this reader and diffs
//! through exactly these tests.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use voxel_core::{Progress, SchoolBBuffer, SparseTree};
use wgpu::util::DeviceExt;

use super::{GpuVoxelizer, map_buffer_u32};
use crate::appearance::AlphaMode;
use crate::bake::{
    ColorCandidate, TexelSource, bake_nearest_owner, srgb_decode_table, srgb_encode_bounds,
};
use crate::core::{MeshInput, VoxelGrid};
use crate::csr::build_brick_csr;
use crate::error::VoxelizeGpuError;
use crate::truecolor::{material_alpha, tri_global_mat};
use voxel_core::MISSING_MAGENTA;

/// Leaves per dispatch: one workgroup per leaf, comfortably under the 65535
/// dispatch-dimension cap; also the progress-meter unit.
const LEAVES_PER_DISPATCH: u32 = 16384;

/// `flags` bit: `wrap_s == Repeat`.
const FLAG_WRAP_S_REPEAT: u32 = 1;
/// `flags` bit: `wrap_t == Repeat`.
const FLAG_WRAP_T_REPEAT: u32 = 1 << 1;
/// `flags` bit: the material is BLEND (keeps its baked alpha).
const FLAG_BLEND: u32 = 1 << 2;
/// `tex` sentinel: the material has no base-colour texture (flat factor).
const NO_TEXTURE: u32 = u32::MAX;

/// One texture's placement in the packed texel buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PackedTextureMeta {
    /// First texel word in [`PackedBakeInputs::texels`].
    pub offset: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Padding (std430 vec4 alignment).
    pub pad: u32,
}

/// One candidate triangle, resolved to grid space with its material data —
/// 96 bytes, 16-byte-aligned fields for a std430 mirror.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PackedTri {
    /// Vertex 0 (grid space).
    pub v0: [f32; 3],
    /// Texture index into the meta table, or `NO_TEXTURE`.
    pub tex: u32,
    /// Vertex 1.
    pub v1: [f32; 3],
    /// Wrap/alpha bit flags (`FLAG_*`).
    pub flags: u32,
    /// Vertex 2.
    pub v2: [f32; 3],
    /// The triangle's global material id (`u16` domain).
    pub global_mat: u32,
    /// Per-vertex base-colour UVs.
    pub uv: [[f32; 2]; 3],
    /// Original triangle index — the argmin tie-break key.
    pub tri_index: u32,
    /// Padding.
    pub pad: u32,
    /// The material's linear `base_color_factor`.
    pub factor: [f32; 4],
}

/// One leaf's slice of the packed world — 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PackedLeaf {
    /// Brick origin (voxel coords).
    pub origin: [u32; 3],
    /// Candidate-range start into [`PackedBakeInputs::cand_indices`].
    pub cand_lo: u32,
    /// Candidate-range end (exclusive).
    pub cand_hi: u32,
    /// This leaf's first slot in the output colour buffer (prefix sum of
    /// occupancy popcounts — the same `leaf_color_base` the renderer reads).
    pub color_base: u32,
    /// Padding.
    pub pad: [u32; 2],
}

/// Every buffer the bake kernel binds, in upload-ready flat form.
pub struct PackedBakeInputs {
    /// All textures' texels, RGBA8 little-endian words, concatenated.
    pub texels: Vec<u32>,
    /// Per-texture placement.
    pub textures: Vec<PackedTextureMeta>,
    /// Per-mesh-triangle candidate records (grid space).
    pub tris: Vec<PackedTri>,
    /// Leaf-grouped candidate triangle ids (indices into `tris`).
    pub cand_indices: Vec<u32>,
    /// Per-leaf origins, candidate ranges, and colour bases.
    pub leaves: Vec<PackedLeaf>,
    /// Leaf occupancy words, 16 × `u32` per leaf (the GPU's `words32` view).
    pub leaf_words: Vec<u32>,
    /// Leaf materials, 256 × `u32` per leaf (two `u16` ids per word, LE).
    pub leaf_mats: Vec<u32>,
    /// Whether the same-material owner constraint applies (a packed material
    /// stream was present at pack time).
    pub material_filter: bool,
    /// Total output colours (== total occupied voxels).
    pub total_colors: u32,
}

/// A window into the packed texel buffer implementing the sampling trait —
/// what the reference reader (and, transcribed, the WGSL kernel) fetches
/// texels through.
pub struct PackedTexelView<'a> {
    texels: &'a [u32],
    meta: PackedTextureMeta,
}

impl TexelSource for PackedTexelView<'_> {
    fn width(&self) -> u32 {
        self.meta.width
    }
    fn height(&self) -> u32 {
        self.meta.height
    }
    fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        self.texels[(self.meta.offset + y * self.meta.width + x) as usize].to_le_bytes()
    }
}

/// Resolves one mesh triangle into its packed candidate record.
#[allow(clippy::cast_possible_truncation)] // triangle counts bounded by the mesh
fn pack_tri(
    ti: usize,
    world: &[Vec3; 3],
    mesh: &MeshInput,
    to_grid: &glam::Mat4,
    packed: Option<&[u32]>,
) -> PackedTri {
    use crate::appearance::WrapMode;
    let appearance = mesh.appearance.as_ref();
    let mat_id = mesh
        .material_ids
        .as_deref()
        .and_then(|m| m.get(ti))
        .copied()
        .unwrap_or(u32::MAX);
    let (tex_index, wrap, factor) = appearance
        .and_then(|app| {
            usize::try_from(mat_id)
                .ok()
                .and_then(|i| app.materials.get(i))
        })
        .map_or(
            (NO_TEXTURE, (WrapMode::Repeat, WrapMode::Repeat), [1.0; 4]),
            |def| {
                (
                    def.base_color_texture.map_or(NO_TEXTURE, |i| i as u32),
                    (def.wrap_s, def.wrap_t),
                    def.base_color_factor,
                )
            },
        );
    let mut flags = 0;
    if wrap.0 == WrapMode::Repeat {
        flags |= FLAG_WRAP_S_REPEAT;
    }
    if wrap.1 == WrapMode::Repeat {
        flags |= FLAG_WRAP_T_REPEAT;
    }
    if material_alpha(appearance, mat_id).0 == AlphaMode::Blend {
        flags |= FLAG_BLEND;
    }
    let uv = mesh
        .uvs
        .as_deref()
        .and_then(|u| u.get(ti))
        .copied()
        .unwrap_or([Vec2::ZERO; 3]);
    let v = [
        to_grid.transform_point3(world[0]),
        to_grid.transform_point3(world[1]),
        to_grid.transform_point3(world[2]),
    ];
    PackedTri {
        v0: v[0].to_array(),
        tex: tex_index,
        v1: v[1].to_array(),
        flags,
        v2: v[2].to_array(),
        global_mat: u32::from(tri_global_mat(packed, ti)),
        uv: [uv[0].to_array(), uv[1].to_array(), uv[2].to_array()],
        tri_index: ti as u32,
        pad: 0,
        factor,
    }
}

/// Flattens everything the bake reads into [`PackedBakeInputs`]. Pure CPU and
/// deterministic; `epsilon` must match the voxelization overlap (the same CSR
/// binning the production bake uses).
#[must_use]
#[allow(clippy::cast_possible_truncation)] // counts bounded by mesh/scene sizes
pub fn pack_bake_inputs(
    tree: &SparseTree,
    structure: &SchoolBBuffer,
    mesh: &MeshInput,
    grid: &VoxelGrid,
    epsilon: f32,
    packed: Option<&[u32]>,
) -> PackedBakeInputs {
    let appearance = mesh.appearance.as_ref();
    let to_grid = grid.world_to_grid_matrix();

    // Textures → one texel word buffer + placement table.
    let mut texels = Vec::new();
    let mut textures = Vec::new();
    if let Some(app) = appearance {
        for tex in &app.textures {
            textures.push(PackedTextureMeta {
                offset: texels.len() as u32,
                width: tex.width(),
                height: tex.height(),
                pad: 0,
            });
            texels.extend(tex.rgba().iter().map(|&t| u32::from_le_bytes(t)));
        }
    }

    // Triangles → grid-space candidate records with resolved material data.
    let tris: Vec<PackedTri> = mesh
        .triangles
        .iter()
        .enumerate()
        .map(|(ti, w)| pack_tri(ti, w, mesh, &to_grid, packed))
        .collect();

    // Leaves → origins, CSR candidate ranges, colour bases, occupancy, materials.
    let csr = build_brick_csr(mesh, grid, 8, epsilon);
    let mut cand_indices = Vec::new();
    let mut leaves = Vec::new();
    let mut leaf_words = Vec::with_capacity(structure.leaves().len() * 16);
    let mut leaf_mats = Vec::with_capacity(structure.leaves().len() * 256);
    let mut color_base = 0u32;
    for (idx, brick) in structure.leaves().iter().enumerate() {
        let origin = tree.leaf_origin(idx);
        // brick_origins hold voxel-coordinate origins (multiples of 8) sorted
        // by (z, y, x) — the leaf origin is already brick-aligned.
        let key = [origin.z, origin.y, origin.x];
        let cand_lo = cand_indices.len() as u32;
        if let Ok(bi) = csr
            .brick_origins
            .binary_search_by(|o| [o[2], o[1], o[0]].cmp(&key))
        {
            let lo = csr.brick_offsets[bi] as usize;
            let hi = csr.brick_offsets[bi + 1] as usize;
            cand_indices.extend_from_slice(&csr.tri_indices[lo..hi]);
        }
        leaves.push(PackedLeaf {
            origin: [origin.x, origin.y, origin.z],
            cand_lo,
            cand_hi: cand_indices.len() as u32,
            color_base,
            pad: [0; 2],
        });
        color_base += brick.count_occupied();
        leaf_words.extend_from_slice(&brick.words32());
        let mats = tree.leaf_materials(idx);
        for pair in mats.chunks_exact(2) {
            leaf_mats.push(u32::from(pair[0]) | (u32::from(pair[1]) << 16));
        }
    }

    PackedBakeInputs {
        texels,
        textures,
        tris,
        cand_indices,
        leaves,
        leaf_words,
        leaf_mats,
        material_filter: packed.is_some(),
        total_colors: color_base,
    }
}

/// The CPU reference reader over the packed buffers — the WGSL kernel's
/// blueprint. Produces the compact colour words in `leaf_color` layout
/// (`color_base + occupied rank`), byte-identical to `bake_leaf_colors` on the
/// same scene (the differential tests pin this).
#[must_use]
#[allow(clippy::cast_possible_truncation)] // morton/local coords < 512
pub fn reference_bake(inputs: &PackedBakeInputs) -> Vec<u32> {
    let views: Vec<PackedTexelView<'_>> = inputs
        .textures
        .iter()
        .map(|&meta| PackedTexelView {
            texels: &inputs.texels,
            meta,
        })
        .collect();
    let wrap_of = |flags: u32, bit: u32| {
        if flags & bit != 0 {
            crate::appearance::WrapMode::Repeat
        } else {
            crate::appearance::WrapMode::ClampToEdge
        }
    };

    let mut out = vec![0u32; inputs.total_colors as usize];
    let mut candidates: Vec<ColorCandidate<'_, PackedTexelView<'_>>> = Vec::new();
    let mut same_mat: Vec<ColorCandidate<'_, PackedTexelView<'_>>> = Vec::new();

    for (li, leaf) in inputs.leaves.iter().enumerate() {
        // Rebuild this leaf's candidate list from the packed records.
        candidates.clear();
        for &ti in &inputs.cand_indices[leaf.cand_lo as usize..leaf.cand_hi as usize] {
            let t = &inputs.tris[ti as usize];
            candidates.push(ColorCandidate {
                tri_index: t.tri_index as usize,
                verts: [
                    Vec3::from_array(t.v0),
                    Vec3::from_array(t.v1),
                    Vec3::from_array(t.v2),
                ],
                uvs: [
                    Vec2::from_array(t.uv[0]),
                    Vec2::from_array(t.uv[1]),
                    Vec2::from_array(t.uv[2]),
                ],
                texture: (t.tex != NO_TEXTURE).then(|| &views[t.tex as usize]),
                wrap: (
                    wrap_of(t.flags, FLAG_WRAP_S_REPEAT),
                    wrap_of(t.flags, FLAG_WRAP_T_REPEAT),
                ),
                factor: t.factor,
            });
        }

        let words = &inputs.leaf_words[li * 16..li * 16 + 16];
        let mats = &inputs.leaf_mats[li * 256..li * 256 + 256];
        let mut slot = leaf.color_base as usize;
        for m in 0..512u32 {
            if words[(m >> 5) as usize] & (1 << (m & 31)) == 0 {
                continue;
            }
            let local = voxel_core::morton::decode(u64::from(m));
            let centre = Vec3::new(
                (leaf.origin[0] + local.x) as f32 + 0.5,
                (leaf.origin[1] + local.y) as f32 + 0.5,
                (leaf.origin[2] + local.z) as f32 + 0.5,
            );
            // Same-material constraint with the fall-back-to-all rule.
            let pick: &[ColorCandidate<'_, PackedTexelView<'_>>] = if inputs.material_filter {
                let vox_mat = (mats[(m >> 1) as usize] >> ((m & 1) * 16)) & 0xFFFF;
                same_mat.clear();
                for c in &candidates {
                    if inputs.tris[c.tri_index].global_mat == vox_mat {
                        same_mat.push(*c);
                    }
                }
                if same_mat.is_empty() {
                    &candidates
                } else {
                    &same_mat
                }
            } else {
                &candidates
            };

            let color = match bake_nearest_owner(centre, pick) {
                Some((i, mut color)) => {
                    let ti = pick[i].tri_index;
                    if inputs.tris[ti].flags & FLAG_BLEND == 0 {
                        color[3] = 255;
                    }
                    u32::from_le_bytes(color)
                }
                None => MISSING_MAGENTA,
            };
            out[slot] = color;
            slot += 1;
        }
    }
    out
}

/// The dispatch uniform (mirrors the WGSL `Uniforms` struct).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BakeUniforms {
    leaf_offset: u32,
    leaf_count: u32,
    material_filter: u32,
    decode_off: u32,
    bounds_off: u32,
    pad: [u32; 3],
}

/// A non-empty storage upload (WGSL bindings reject zero-sized buffers; a
/// single zero word is never read when the logical length is zero).
fn storage_words(device: &wgpu::Device, label: &str, words: &[u32]) -> wgpu::Buffer {
    let one = [0u32];
    let data: &[u32] = if words.is_empty() { &one } else { words };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

impl GpuVoxelizer {
    /// Runs the colour bake on the GPU over [`PackedBakeInputs`], returning
    /// the compact `leaf_color` words — the same layout, and (pinned by the
    /// differential tests) the same bytes, as `bake_leaf_colors` /
    /// [`reference_bake`]. Chunked one-workgroup-per-leaf dispatches; each
    /// chunk ticks `progress`.
    ///
    /// # Errors
    /// [`VoxelizeGpuError::PipelineValidation`] when an input buffer exceeds
    /// the device's storage-binding cap (the caller falls back to the CPU
    /// bake).
    #[allow(clippy::too_many_lines)] // one-shot GPU setup: buffers + pipeline + chunked dispatch
    pub async fn bake_leaf_colors_gpu(
        &self,
        inputs: &PackedBakeInputs,
        progress: &mut Progress<'_>,
    ) -> Result<Vec<u32>, VoxelizeGpuError> {
        if inputs.total_colors == 0 {
            return Ok(Vec::new());
        }
        let device = self.device();
        let queue = self.queue();

        // aux = [texture meta | sRGB decode bits | encode-bound bits].
        let mut aux: Vec<u32> = Vec::new();
        for meta in &inputs.textures {
            aux.extend_from_slice(&[meta.offset, meta.width, meta.height, 0]);
        }
        let decode_off = aux.len() as u32;
        aux.extend(srgb_decode_table().iter().map(|f| f.to_bits()));
        let bounds_off = aux.len() as u32;
        aux.extend(srgb_encode_bounds().iter().map(|f| f.to_bits()));

        let out_bytes = u64::from(inputs.total_colors) * 4;
        let tri_bytes = (inputs.tris.len() * std::mem::size_of::<PackedTri>()) as u64;
        let texel_bytes = (inputs.texels.len() * 4) as u64;
        let cap = device.limits().max_storage_buffer_binding_size;
        for (label, bytes) in [
            ("colour output", out_bytes),
            ("triangles", tri_bytes),
            ("texels", texel_bytes),
        ] {
            if bytes > cap {
                return Err(VoxelizeGpuError::PipelineValidation(format!(
                    "gpu bake: {label} buffer ({bytes} B) exceeds the storage-binding cap ({cap} B)"
                )));
            }
        }

        let texels_buf = storage_words(device, "bake texels", &inputs.texels);
        let aux_buf = storage_words(device, "bake aux", &aux);
        let zero_tri = [PackedTri::zeroed()];
        let tris_slice: &[PackedTri] = if inputs.tris.is_empty() {
            &zero_tri
        } else {
            &inputs.tris
        };
        let tris_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bake tris"),
            contents: bytemuck::cast_slice(tris_slice),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let cand_buf = storage_words(device, "bake cand indices", &inputs.cand_indices);
        let leaves_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bake leaves"),
            contents: bytemuck::cast_slice(&inputs.leaves),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let words_buf = storage_words(device, "bake leaf words", &inputs.leaf_words);
        let mats_buf = storage_words(device, "bake leaf mats", &inputs.leaf_mats);
        let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake out"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake uniforms"),
            size: std::mem::size_of::<BakeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("colorbake"),
            source: wgpu::ShaderSource::Wgsl(super::shaders::COLORBAKE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("colorbake"),
            layout: None,
            module: &module,
            entry_point: Some("bake"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("colorbake bind"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: texels_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: aux_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tris_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: cand_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: leaves_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: words_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: mats_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: uniform_buf.as_entire_binding(),
                },
            ],
        });

        // Chunked dispatch: one workgroup per leaf, one meter tick per chunk.
        let leaf_count = inputs.leaves.len() as u32;
        let mut meter = progress.meter(u64::from(leaf_count.div_ceil(LEAVES_PER_DISPATCH)));
        let mut offset = 0u32;
        while offset < leaf_count {
            let chunk = (leaf_count - offset).min(LEAVES_PER_DISPATCH);
            queue.write_buffer(
                &uniform_buf,
                0,
                bytemuck::bytes_of(&BakeUniforms {
                    leaf_offset: offset,
                    leaf_count: chunk,
                    material_filter: u32::from(inputs.material_filter),
                    decode_off,
                    bounds_off,
                    pad: [0; 3],
                }),
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("colorbake chunk"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("colorbake"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(chunk, 1, 1);
            }
            queue.submit([encoder.finish()]);
            meter.add(1);
            offset += chunk;
        }

        // Read back the compact colour words.
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bake readback"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("colorbake readback"),
        });
        encoder.copy_buffer_to_buffer(&out_buf, 0, &readback, 0, out_bytes);
        queue.submit([encoder.finish()]);
        map_buffer_u32(&readback, device).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::{MaterialDef, MeshAppearance, Texture, WrapMode};
    use crate::materials::material_table_for_sparse;
    use crate::truecolor::bake_leaf_colors;
    use voxel_core::{Resolution, VoxelCoord};

    /// A 4×4 gradient so bilinear taps and REPEAT wraps have structure to
    /// disagree over if the packing were wrong.
    fn gradient(seed: u8) -> Texture {
        let mut px = Vec::with_capacity(16);
        for i in 0..16u8 {
            px.push([
                seed.wrapping_mul(3).wrapping_add(i * 16),
                i * 15,
                255 - i * 12,
                255,
            ]);
        }
        Texture::new(4, 4, px).expect("4x4 texture")
    }

    /// The full-model differential: voxelize `LittlestTokyo` on the GPU, bake on
    /// the CPU oracle, then GPU-bake the packed inputs and compare every voxel
    /// word.
    ///
    /// **Policy:** the synthetic differentials assert bit-exactness (the sRGB
    /// tables + transcribed arithmetic make that achievable where geometry is
    /// controlled). On a real model a small fraction of voxels legitimately
    /// diverges: WGSL permits fused multiply-add in the dot products, so
    /// near-tie owner picks (two triangles equidistant to within a ULP) and
    /// bilinear tap boundaries can resolve differently — both answers are
    /// "the nearest surface" within the algorithm's own uncertainty. Measured
    /// on this model: 344 of 133 252 (0.26%), about half within ±2/channel
    /// blend drift, the rest near-tie owner flips. The assertion bounds the
    /// fraction at 1%. Skips without an adapter, the `gltf` feature's loader,
    /// or the local reference model.
    #[cfg(feature = "gltf")]
    #[test]
    fn gpu_bake_matches_cpu_on_littlest_tokyo() {
        use voxel_core::Resolution;

        let Ok(gpu) = pollster::block_on(GpuVoxelizer::new_standalone(
            super::super::GpuVoxelizerConfig::default(),
        )) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../models/gltf/LittlestTokyo.glb"
        );
        let Ok(mesh) = crate::load_gltf_path(path) else {
            eprintln!("skipping: reference model not present ({path})");
            return;
        };

        let resolution = Resolution::new(128).expect("legal");
        let grid = VoxelGrid::fit_mesh(resolution, &mesh, 2.0);
        let opts = crate::core::VoxelizeOpts {
            epsilon: 1e-4,
            store_owner: true,
            store_color: false,
        };
        let (_table, packed) = material_table_for_sparse(&mesh).expect("materials");
        let voxels =
            pollster::block_on(gpu.compact_surface_sparse(&mesh, &grid, &opts, &packed, [0, 0, 0]))
                .expect("voxelize");
        let voxels = crate::truecolor::cull_mask_cutout(
            &voxels,
            &mesh,
            &grid,
            opts.epsilon,
            Some(&packed),
            &mut voxel_core::Progress::none(),
        );
        let (tree, _dropped) = crate::materials::tree_from_compact(
            resolution,
            &voxels,
            &mut voxel_core::Progress::none(),
        );
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        bake_leaf_colors(
            &mut structure,
            &tree,
            &mesh,
            &grid,
            opts.epsilon,
            Some(&packed),
            &mut voxel_core::Progress::none(),
        );

        let inputs = pack_bake_inputs(&tree, &structure, &mesh, &grid, opts.epsilon, Some(&packed));
        let got = pollster::block_on(
            gpu.bake_leaf_colors_gpu(&inputs, &mut voxel_core::Progress::none()),
        )
        .expect("gpu bake");

        let want = structure.leaf_color_words();
        assert_eq!(got.len(), want.len());
        let mut mismatches = 0usize;
        let mut small = 0usize; // every channel within ±2 (ULP-scale blend drift)
        let mut max_delta = 0u32;
        for (&g, &w) in got.iter().zip(want.iter()) {
            if g == w {
                continue;
            }
            mismatches += 1;
            let delta = g
                .to_le_bytes()
                .iter()
                .zip(w.to_le_bytes().iter())
                .map(|(a, b)| u32::from(a.abs_diff(*b)))
                .max()
                .unwrap_or(0);
            max_delta = max_delta.max(delta);
            if delta <= 2 {
                small += 1;
            }
        }
        eprintln!(
            "tokyo gpu-vs-cpu: {mismatches} of {} differ; {small} within ±2/channel; max channel delta {max_delta}",
            want.len()
        );
        assert!(
            mismatches * 100 <= want.len(),
            "GPU bake diverged on {mismatches} of {} voxels (> 1%) — beyond \
             near-tie float-contraction territory; investigate before trusting",
            want.len()
        );
    }

    /// Baked scene bundle shared by the packed-reference and GPU differentials.
    struct BakedScene {
        inputs: PackedBakeInputs,
        structure: SchoolBBuffer,
    }

    /// The GPU kernel must reproduce the CPU bake word-for-word on both
    /// differential scenes. Skips without an adapter (workspace convention).
    #[test]
    fn gpu_bake_matches_cpu_on_differential_scenes() {
        let Ok(gpu) = pollster::block_on(GpuVoxelizer::new_standalone(
            super::super::GpuVoxelizerConfig::default(),
        )) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        for (name, scene) in [
            ("multi-material", build_multi_material_scene()),
            ("no-filter", build_no_filter_scene()),
        ] {
            let got = pollster::block_on(
                gpu.bake_leaf_colors_gpu(&scene.inputs, &mut voxel_core::Progress::none()),
            )
            .expect("gpu bake");
            assert_eq!(
                got,
                scene.structure.leaf_color_words(),
                "GPU bake diverged from the CPU oracle on the {name} scene"
            );
        }
    }

    /// The packed reference must reproduce `bake_leaf_colors` word-for-word on
    /// a scene exercising: two textures of different materials, REPEAT and
    /// CLAMP wraps, non-unit factors, a minifying UV scale (supersampled
    /// path), the same-material fallback, and an untextured material. This is
    /// the differential the WGSL kernel runs through too.
    #[test]
    fn packed_reference_matches_production_bake() {
        let scene = build_multi_material_scene();
        let got = reference_bake(&scene.inputs);
        assert_eq!(
            got,
            scene.structure.leaf_color_words(),
            "packed reference diverged from the production bake"
        );
    }

    /// Builds + CPU-bakes the multi-material differential scene.
    #[allow(clippy::too_many_lines)] // one scene definition
    fn build_multi_material_scene() -> BakedScene {
        let r = Resolution::new(32).unwrap();
        let grid = VoxelGrid::new(r, Vec3::ZERO, 1.0);
        let uv_small = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
        ];
        // Large UV scale → minification → the supersampled filter path.
        let uv_minified = [
            Vec2::new(0.0, 0.0),
            Vec2::new(8.0, 0.0),
            Vec2::new(0.0, 8.0),
        ];
        let tris = vec![
            [
                Vec3::new(0.0, 0.0, 2.0),
                Vec3::new(20.0, 0.0, 2.0),
                Vec3::new(0.0, 20.0, 2.0),
            ],
            [
                Vec3::new(0.0, 0.0, 6.0),
                Vec3::new(20.0, 0.0, 6.0),
                Vec3::new(0.0, 20.0, 6.0),
            ],
            [
                Vec3::new(0.0, 0.0, 10.0),
                Vec3::new(20.0, 0.0, 10.0),
                Vec3::new(0.0, 20.0, 10.0),
            ],
        ];
        let mesh = MeshInput {
            triangles: tris,
            material_ids: Some(vec![0, 1, 2]),
            uvs: Some(vec![uv_small, uv_minified, uv_small]),
            appearance: Some(MeshAppearance {
                textures: vec![gradient(1), gradient(7)],
                materials: vec![
                    MaterialDef {
                        name: None,
                        base_color_texture: Some(0),
                        base_color_factor: [1.0, 0.75, 0.5, 1.0],
                        wrap_s: WrapMode::ClampToEdge,
                        wrap_t: WrapMode::ClampToEdge,
                        alpha_mode: AlphaMode::Opaque,
                        alpha_cutoff: 0.5,
                    },
                    MaterialDef {
                        name: None,
                        base_color_texture: Some(1),
                        base_color_factor: [1.0, 1.0, 1.0, 0.8],
                        wrap_s: WrapMode::Repeat,
                        wrap_t: WrapMode::Repeat,
                        alpha_mode: AlphaMode::Blend,
                        alpha_cutoff: 0.5,
                    },
                    MaterialDef {
                        name: None,
                        base_color_texture: None,
                        base_color_factor: [0.25, 0.5, 0.75, 1.0],
                        wrap_s: WrapMode::Repeat,
                        wrap_t: WrapMode::Repeat,
                        alpha_mode: AlphaMode::Opaque,
                        alpha_cutoff: 0.5,
                    },
                ],
            }),
        };
        let (_table, packed) = material_table_for_sparse(&mesh).expect("pack materials");
        let global_of = |ti: usize| tri_global_mat(Some(&packed), ti);

        // Voxels across the three surfaces + one whose material has no
        // candidate in its brick (the fallback rule), spread over 4 leaves.
        let voxels = [
            (VoxelCoord::new(1, 1, 2), global_of(0)),
            (VoxelCoord::new(9, 3, 2), global_of(0)),
            (VoxelCoord::new(2, 2, 6), global_of(1)),
            (VoxelCoord::new(11, 5, 6), global_of(1)),
            (VoxelCoord::new(3, 3, 10), global_of(2)),
            (VoxelCoord::new(12, 9, 10), global_of(2)),
            (VoxelCoord::new(5, 5, 3), 777u16), // no such material → fallback
        ];
        let tree = SparseTree::from_voxels(r, voxels.iter().copied());
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        bake_leaf_colors(
            &mut structure,
            &tree,
            &mesh,
            &grid,
            0.5,
            Some(&packed),
            &mut voxel_core::Progress::none(),
        );

        let inputs = pack_bake_inputs(&tree, &structure, &mesh, &grid, 0.5, Some(&packed));
        assert_eq!(inputs.total_colors as usize, voxels.len());
        // Bases must agree with the assembler's prefix sums.
        for (li, leaf) in inputs.leaves.iter().enumerate() {
            assert_eq!(
                leaf.color_base,
                structure.leaf_color_base_words()[li],
                "leaf {li} base diverged"
            );
        }
        BakedScene { inputs, structure }
    }

    /// Without a packed material stream the filter is off — parity must hold
    /// down that arm too.
    #[test]
    fn packed_reference_matches_without_material_filter() {
        let scene = build_no_filter_scene();
        assert_eq!(
            reference_bake(&scene.inputs),
            scene.structure.leaf_color_words()
        );
    }

    /// Builds + CPU-bakes the filter-off differential scene.
    fn build_no_filter_scene() -> BakedScene {
        let r = Resolution::new(8).unwrap();
        let grid = VoxelGrid::new(r, Vec3::ZERO, 1.0);
        let mesh = MeshInput {
            triangles: vec![[
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(8.0, 0.0, 1.0),
                Vec3::new(0.0, 8.0, 1.0),
            ]],
            material_ids: Some(vec![0]),
            uvs: Some(vec![[
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
            ]]),
            appearance: Some(MeshAppearance {
                textures: vec![gradient(3)],
                materials: vec![MaterialDef {
                    name: None,
                    base_color_texture: Some(0),
                    base_color_factor: [1.0; 4],
                    wrap_s: WrapMode::ClampToEdge,
                    wrap_t: WrapMode::ClampToEdge,
                    alpha_mode: AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                }],
            }),
        };
        let voxels = [
            (VoxelCoord::new(1, 1, 1), 0u16),
            (VoxelCoord::new(4, 2, 1), 0u16),
        ];
        let tree = SparseTree::from_voxels(r, voxels.iter().copied());
        let mut structure = SchoolBBuffer::from_sparse(&tree);
        bake_leaf_colors(
            &mut structure,
            &tree,
            &mesh,
            &grid,
            0.0,
            None,
            &mut voxel_core::Progress::none(),
        );

        let inputs = pack_bake_inputs(&tree, &structure, &mesh, &grid, 0.0, None);
        assert!(!inputs.material_filter);
        BakedScene { inputs, structure }
    }
}
