use glam::Vec3;

use crate::core::{MeshInput, TileSpec, VoxelGridSpec};

#[derive(Debug, Clone)]
pub struct TileTriangleCsr {
    pub tile_offsets: Vec<u32>,
    pub tri_indices: Vec<u32>,
    pub tri_counts: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct BrickTriangleCsr {
    pub brick_origins: Vec<[u32; 3]>,
    pub brick_offsets: Vec<u32>,
    pub tri_indices: Vec<u32>,
}

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
