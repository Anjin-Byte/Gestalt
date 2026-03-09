//! Compressed sparse row (CSR) builders for triangle-to-cell assignment.
//! (Conservative Uniform-Grid Binning with CSR (CUGB-CSR))
//! This module precomputes spatial lookup tables that map grid partitions
//! (tiles or bricks) to candidate triangle indices. The resulting CSR
//! structures are used by voxelization paths to limit intersection work to
//! relevant triangles per partition.
//!
//! Two partitioning schemes are supported:
//! - [`TileTriangleCsr`]: regular tile grid from [`TileSpec`]
//! - [`BrickTriangleCsr`]: sparse brick set derived from triangle coverage
//!
//! Both outputs follow the standard CSR shape:
//! - `*_offsets.len() == cell_count + 1`
//! - for cell `i`, triangle range is `tri_indices[offsets[i]..offsets[i + 1]]`
//! - `offsets[0] == 0`
//! - `offsets` is monotonic non-decreasing
//! - `offsets.last() == tri_indices.len()`

use glam::Vec3;

use crate::core::{MeshInput, TileSpec, VoxelGridSpec};

/// CSR mapping from regular tiles to candidate triangles.
///
/// Tile indexing uses X-major layout:
/// `tile = tx + nx * ty + nx * ny * tz`.
#[derive(Debug, Clone)]
pub struct TileTriangleCsr {
    /// Prefix-sum offsets into [`Self::tri_indices`], length = tile_count + 1.
    pub tile_offsets: Vec<u32>,
    /// Flattened triangle index list for all tiles.
    pub tri_indices: Vec<u32>,
    /// Number of triangle references per tile (same cardinality as tile_count).
    ///
    /// This is equivalent to `tile_offsets[i + 1] - tile_offsets[i]`.
    pub tri_counts: Vec<u32>,
}

/// CSR mapping from sparse bricks to candidate triangles.
///
/// Unlike [`TileTriangleCsr`], only bricks touched by at least one triangle
/// are emitted.
#[derive(Debug, Clone)]
pub struct BrickTriangleCsr {
    /// World-grid origins for each emitted brick, sorted by `(z, y, x)`.
    pub brick_origins: Vec<[u32; 3]>,
    /// Prefix-sum offsets into [`Self::tri_indices`], length = brick_count + 1.
    pub brick_offsets: Vec<u32>,
    /// Flattened triangle index list for all emitted bricks.
    pub tri_indices: Vec<u32>,
}

/// Build a dense tile CSR over the full tile lattice.
///
/// Each triangle is transformed into grid space, expanded by `epsilon`, and
/// assigned to every overlapping tile in the regular tile grid.
///
/// This function performs a standard two-pass CSR build:
/// 1. Count references per tile.
/// 2. Prefix-sum counts to offsets, then scatter triangle indices.
///
/// # Parameters
/// - `mesh`: source triangles in world space.
/// - `grid`: voxel grid transform and bounds.
/// - `tiles`: tile dimensions and lattice shape.
/// - `epsilon`: conservative expansion margin in grid units.
///
/// # Returns
/// A [`TileTriangleCsr`] that includes all tiles from `tiles`.
pub fn build_tile_csr(
    mesh: &MeshInput,
    grid: &VoxelGridSpec,
    tiles: &TileSpec,
    epsilon: f32,
) -> TileTriangleCsr {
    let num_tiles_total = tiles.num_tiles_total() as usize;
    let mut counts = vec![0u32; num_tiles_total];
    let to_grid = grid.world_to_grid_matrix();

    for (_tri_index, tri) in mesh.triangles.iter().enumerate() {
        let v0 = to_grid.transform_point3(tri[0]);
        let v1 = to_grid.transform_point3(tri[1]);
        let v2 = to_grid.transform_point3(tri[2]);
        let min_v = v0.min(v1).min(v2) - Vec3::splat(epsilon);
        let max_v = v0.max(v1).max(v2) + Vec3::splat(epsilon);

        let max_x = grid.dims[0].saturating_sub(1) as i32;
        let max_y = grid.dims[1].saturating_sub(1) as i32;
        let max_z = grid.dims[2].saturating_sub(1) as i32;
        let min_voxel = [
            min_v.x.floor() as i32,
            min_v.y.floor() as i32,
            min_v.z.floor() as i32,
        ];
        let max_voxel = [
            max_v.x.floor() as i32,
            max_v.y.floor() as i32,
            max_v.z.floor() as i32,
        ];
        let min_voxel = [
            min_voxel[0].clamp(0, max_x),
            min_voxel[1].clamp(0, max_y),
            min_voxel[2].clamp(0, max_z),
        ];
        let max_voxel = [
            max_voxel[0].clamp(0, max_x),
            max_voxel[1].clamp(0, max_y),
            max_voxel[2].clamp(0, max_z),
        ];

        let min_tile = [
            (min_voxel[0].div_euclid(tiles.tile_dims[0] as i32))
                .clamp(0, tiles.num_tiles[0] as i32 - 1),
            (min_voxel[1].div_euclid(tiles.tile_dims[1] as i32))
                .clamp(0, tiles.num_tiles[1] as i32 - 1),
            (min_voxel[2].div_euclid(tiles.tile_dims[2] as i32))
                .clamp(0, tiles.num_tiles[2] as i32 - 1),
        ];
        let max_tile = [
            (max_voxel[0].div_euclid(tiles.tile_dims[0] as i32))
                .clamp(0, tiles.num_tiles[0] as i32 - 1),
            (max_voxel[1].div_euclid(tiles.tile_dims[1] as i32))
                .clamp(0, tiles.num_tiles[1] as i32 - 1),
            (max_voxel[2].div_euclid(tiles.tile_dims[2] as i32))
                .clamp(0, tiles.num_tiles[2] as i32 - 1),
        ];

        if min_tile[0] > max_tile[0] || min_tile[1] > max_tile[1] || min_tile[2] > max_tile[2] {
            continue;
        }

        for tz in min_tile[2]..=max_tile[2] {
            for ty in min_tile[1]..=max_tile[1] {
                for tx in min_tile[0]..=max_tile[0] {
                    let tile_index = (tx as u32)
                        + tiles.num_tiles[0] * (ty as u32)
                        + tiles.num_tiles[0] * tiles.num_tiles[1] * (tz as u32);
                    counts[tile_index as usize] += 1;
                }
            }
        }
    }

    let mut offsets = vec![0u32; num_tiles_total + 1];
    for i in 0..num_tiles_total {
        offsets[i + 1] = offsets[i] + counts[i];
    }

    let mut cursor = offsets.clone();
    let mut tri_indices = vec![0u32; offsets[num_tiles_total] as usize];
    for (tri_index, tri) in mesh.triangles.iter().enumerate() {
        let v0 = to_grid.transform_point3(tri[0]);
        let v1 = to_grid.transform_point3(tri[1]);
        let v2 = to_grid.transform_point3(tri[2]);
        let min_v = v0.min(v1).min(v2) - Vec3::splat(epsilon);
        let max_v = v0.max(v1).max(v2) + Vec3::splat(epsilon);

        let min_voxel = [
            min_v.x.floor() as i32,
            min_v.y.floor() as i32,
            min_v.z.floor() as i32,
        ];
        let max_voxel = [
            max_v.x.floor() as i32,
            max_v.y.floor() as i32,
            max_v.z.floor() as i32,
        ];

        let min_tile = [
            (min_voxel[0].div_euclid(tiles.tile_dims[0] as i32))
                .clamp(0, tiles.num_tiles[0] as i32 - 1),
            (min_voxel[1].div_euclid(tiles.tile_dims[1] as i32))
                .clamp(0, tiles.num_tiles[1] as i32 - 1),
            (min_voxel[2].div_euclid(tiles.tile_dims[2] as i32))
                .clamp(0, tiles.num_tiles[2] as i32 - 1),
        ];
        let max_tile = [
            (max_voxel[0].div_euclid(tiles.tile_dims[0] as i32))
                .clamp(0, tiles.num_tiles[0] as i32 - 1),
            (max_voxel[1].div_euclid(tiles.tile_dims[1] as i32))
                .clamp(0, tiles.num_tiles[1] as i32 - 1),
            (max_voxel[2].div_euclid(tiles.tile_dims[2] as i32))
                .clamp(0, tiles.num_tiles[2] as i32 - 1),
        ];

        if min_tile[0] > max_tile[0] || min_tile[1] > max_tile[1] || min_tile[2] > max_tile[2] {
            continue;
        }

        for tz in min_tile[2]..=max_tile[2] {
            for ty in min_tile[1]..=max_tile[1] {
                for tx in min_tile[0]..=max_tile[0] {
                    let tile_index = (tx as u32)
                        + tiles.num_tiles[0] * (ty as u32)
                        + tiles.num_tiles[0] * tiles.num_tiles[1] * (tz as u32);
                    let write = cursor[tile_index as usize];
                    tri_indices[write as usize] = tri_index as u32;
                    cursor[tile_index as usize] += 1;
                }
            }
        }
    }

    TileTriangleCsr {
        tile_offsets: offsets,
        tri_indices,
        tri_counts: counts,
    }
}

/// Build a sparse brick CSR from triangle coverage.
///
/// Triangles are transformed into grid space, expanded by `epsilon`, and
/// associated with every overlapping brick of size `brick_dim^3`.
/// Only bricks with at least one triangle are emitted.
///
/// # Parameters
/// - `mesh`: source triangles in world space.
/// - `grid`: voxel grid transform and bounds.
/// - `brick_dim`: edge length of each brick in voxels.
/// - `epsilon`: conservative expansion margin in grid units.
///
/// # Returns
/// A [`BrickTriangleCsr`] where:
/// - `brick_origins` are sorted by `(z, y, x)` for stable iteration order.
/// - `brick_offsets` and `tri_indices` follow CSR invariants.
pub fn build_brick_csr(
    mesh: &MeshInput,
    grid: &VoxelGridSpec,
    brick_dim: u32,
    epsilon: f32,
) -> BrickTriangleCsr {
    use std::collections::HashMap;

    let to_grid = grid.world_to_grid_matrix();
    let mut brick_map: HashMap<(u32, u32, u32), Vec<u32>> = HashMap::new();

    for (tri_index, tri) in mesh.triangles.iter().enumerate() {
        let v0 = to_grid.transform_point3(tri[0]);
        let v1 = to_grid.transform_point3(tri[1]);
        let v2 = to_grid.transform_point3(tri[2]);
        let min_v = v0.min(v1).min(v2) - Vec3::splat(epsilon);
        let max_v = v0.max(v1).max(v2) + Vec3::splat(epsilon);

        let min_voxel = [
            min_v.x.floor() as i32,
            min_v.y.floor() as i32,
            min_v.z.floor() as i32,
        ];
        let max_voxel = [
            max_v.x.floor() as i32,
            max_v.y.floor() as i32,
            max_v.z.floor() as i32,
        ];

        let min_brick = [
            (min_voxel[0].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
            (min_voxel[1].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
            (min_voxel[2].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
        ];
        let max_brick = [
            (max_voxel[0].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
            (max_voxel[1].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
            (max_voxel[2].div_euclid(brick_dim as i32)).clamp(0, i32::MAX),
        ];

        if min_brick[0] > max_brick[0] || min_brick[1] > max_brick[1] || min_brick[2] > max_brick[2] {
            continue;
        }

        for bz in min_brick[2]..=max_brick[2] {
            for by in min_brick[1]..=max_brick[1] {
                for bx in min_brick[0]..=max_brick[0] {
                    let key = (bx as u32, by as u32, bz as u32);
                    brick_map.entry(key).or_default().push(tri_index as u32);
                }
            }
        }
    }

    // Diagnostic: report CSR memory usage
    let total_refs: usize = brick_map.values().map(|v| v.len()).sum();
    let brick_count = brick_map.len();
    // HashMap overhead: ~64 bytes per entry + Vec overhead per bucket
    let estimated_bytes = brick_count * 80 + total_refs * 4;
    #[cfg(target_arch = "wasm32")]
    {
        let msg = format!(
            "[build_brick_csr] bricks={}, tri_refs={}, est_memory={}MB, triangles={}, grid_dims={:?}, brick_dim={}",
            brick_count, total_refs, estimated_bytes / (1024 * 1024),
            mesh.triangles.len(), grid.dims, brick_dim
        );
        web_sys::console::log_1(&msg.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (brick_count, total_refs, estimated_bytes);
    }

    let mut brick_origins: Vec<[u32; 3]> = brick_map
        .keys()
        .map(|(x, y, z)| [x * brick_dim, y * brick_dim, z * brick_dim])
        .collect();
    brick_origins.sort_by_key(|origin| (origin[2], origin[1], origin[0]));

    let mut brick_offsets = Vec::with_capacity(brick_origins.len() + 1);
    let mut tri_indices = Vec::new();
    brick_offsets.push(0);
    for origin in &brick_origins {
        let key = (origin[0] / brick_dim, origin[1] / brick_dim, origin[2] / brick_dim);
        if let Some(list) = brick_map.get(&key) {
            tri_indices.extend(list.iter().copied());
        }
        brick_offsets.push(tri_indices.len() as u32);
    }

    BrickTriangleCsr {
        brick_origins,
        brick_offsets,
        tri_indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::core::{MeshInput, TileSpec, VoxelGridSpec};

    #[test]
    fn csr_invariants_hold() {
        let grid = VoxelGridSpec {
            origin_world: Vec3::ZERO,
            voxel_size: 1.0,
            dims: [8, 8, 8],
            world_to_grid: None,
        };
        let tiles = TileSpec::new([4, 4, 4], grid.dims).expect("tiles");
        let mesh = MeshInput {
            triangles: vec![
                [Vec3::new(0.1, 0.1, 0.1), Vec3::new(1.2, 0.1, 0.1), Vec3::new(0.1, 1.2, 0.1)],
                [Vec3::new(4.0, 4.0, 4.0), Vec3::new(5.0, 4.0, 4.0), Vec3::new(4.0, 5.0, 4.0)],
            ],
            material_ids: None,
        };
        let csr = build_tile_csr(&mesh, &grid, &tiles, 1e-4);
        assert_eq!(csr.tile_offsets[0], 0);
        for window in csr.tile_offsets.windows(2) {
            assert!(window[0] <= window[1]);
        }
        assert_eq!(csr.tile_offsets.last().copied().unwrap(), csr.tri_indices.len() as u32);
    }
}
