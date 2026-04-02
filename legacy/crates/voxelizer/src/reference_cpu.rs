use glam::Vec3;

use crate::core::{DispatchStats, MeshInput, TileSpec, VoxelGridSpec, VoxelizationOutput, VoxelizeOpts};

fn triangle_box_overlap(box_center: Vec3, box_half: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> bool {
    let v0 = v0 - box_center;
    let v1 = v1 - box_center;
    let v2 = v2 - box_center;

    let e0 = v1 - v0;
    let e1 = v2 - v1;
    let e2 = v0 - v2;

    let axes = [
        Vec3::new(0.0, -e0.z, e0.y),
        Vec3::new(0.0, -e1.z, e1.y),
        Vec3::new(0.0, -e2.z, e2.y),
        Vec3::new(e0.z, 0.0, -e0.x),
        Vec3::new(e1.z, 0.0, -e1.x),
        Vec3::new(e2.z, 0.0, -e2.x),
        Vec3::new(-e0.y, e0.x, 0.0),
        Vec3::new(-e1.y, e1.x, 0.0),
        Vec3::new(-e2.y, e2.x, 0.0),
    ];

    for axis in axes.iter() {
        let p0 = v0.dot(*axis);
        let p1 = v1.dot(*axis);
        let p2 = v2.dot(*axis);
        let min_p = p0.min(p1.min(p2));
        let max_p = p0.max(p1.max(p2));
        let r = box_half.x * axis.x.abs() + box_half.y * axis.y.abs() + box_half.z * axis.z.abs();
        if min_p > r || max_p < -r {
            return false;
        }
    }

    if v0.x.min(v1.x.min(v2.x)) > box_half.x
        || v0.x.max(v1.x.max(v2.x)) < -box_half.x
        || v0.y.min(v1.y.min(v2.y)) > box_half.y
        || v0.y.max(v1.y.max(v2.y)) < -box_half.y
        || v0.z.min(v1.z.min(v2.z)) > box_half.z
        || v0.z.max(v1.z.max(v2.z)) < -box_half.z
    {
        return false;
    }

    let normal = e0.cross(e1);
    let d = -normal.dot(v0);
    let r = box_half.x * normal.x.abs() + box_half.y * normal.y.abs() + box_half.z * normal.z.abs();
    let s = normal.dot(Vec3::ZERO) + d;
    if s.abs() > r {
        return false;
    }

    true
}

fn hash_color(id: u32) -> u32 {
    let mut x = id.wrapping_mul(1664525).wrapping_add(1013904223);
    let r = (x & 0xff) as u8;
    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
    let g = (x & 0xff) as u8;
    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
    let b = (x & 0xff) as u8;
    u32::from_le_bytes([r, g, b, 255])
}

pub fn voxelize_surface_cpu(
    mesh: &MeshInput,
    grid: &VoxelGridSpec,
    _tiles: &TileSpec,
    opts: &VoxelizeOpts,
) -> VoxelizationOutput {
    let dims = grid.dims;
    let num_voxels = (dims[0] as usize) * (dims[1] as usize) * (dims[2] as usize);
    let word_count = (num_voxels + 31) / 32;
    let mut occupancy = vec![0u32; word_count];
    let mut owner = if opts.store_owner { vec![u32::MAX; num_voxels] } else { Vec::new() };
    let mut color = if opts.store_color { vec![0u32; num_voxels] } else { Vec::new() };

    let to_grid = grid.world_to_grid_matrix();
    let half = Vec3::splat(0.5);

    for (tri_index, tri) in mesh.triangles.iter().enumerate() {
        let v0 = to_grid.transform_point3(tri[0]);
        let v1 = to_grid.transform_point3(tri[1]);
        let v2 = to_grid.transform_point3(tri[2]);

        let min_v = v0.min(v1).min(v2) - Vec3::splat(opts.epsilon);
        let max_v = v0.max(v1).max(v2) + Vec3::splat(opts.epsilon);
        let min = [
            min_v.x.floor().max(0.0) as i32,
            min_v.y.floor().max(0.0) as i32,
            min_v.z.floor().max(0.0) as i32,
        ];
        let max = [
            max_v.x.floor().min((dims[0] - 1) as f32) as i32,
            max_v.y.floor().min((dims[1] - 1) as f32) as i32,
            max_v.z.floor().min((dims[2] - 1) as f32) as i32,
        ];

        for z in min[2]..=max[2] {
            for y in min[1]..=max[1] {
                for x in min[0]..=max[0] {
                    let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    if triangle_box_overlap(center, half, v0, v1, v2) {
                        let linear = (x as usize)
                            + (dims[0] as usize) * ((y as usize) + (dims[1] as usize) * (z as usize));
                        let word = linear >> 5;
                        let bit = linear & 31;
                        occupancy[word] |= 1u32 << bit;
                        if opts.store_owner {
                            let owner_ref = &mut owner[linear];
                            let tri_u = tri_index as u32;
                            if tri_u < *owner_ref {
                                *owner_ref = tri_u;
                            }
                        }
                    }
                }
            }
        }
    }

    if opts.store_color {
        for (index, color_out) in color.iter_mut().enumerate() {
            let owner_id = if opts.store_owner { owner[index] } else { u32::MAX };
            if owner_id != u32::MAX {
                *color_out = hash_color(owner_id);
            }
        }
    }

    VoxelizationOutput {
        occupancy,
        owner_id: if opts.store_owner { Some(owner) } else { None },
        color_rgba: if opts.store_color { Some(color) } else { None },
        stats: DispatchStats {
            triangles: mesh.triangles.len() as u32,
            tiles: _tiles.num_tiles_total(),
            voxels: num_voxels as u32,
            gpu_time_ms: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSpec;

    #[test]
    fn cpu_voxelizes_single_triangle() {
        let grid = VoxelGridSpec {
            origin_world: Vec3::ZERO,
            voxel_size: 1.0,
            dims: [4, 4, 4],
            world_to_grid: None,
        };
        let tiles = TileSpec::new([2, 2, 2], grid.dims).expect("tiles");
        let mesh = MeshInput {
            triangles: vec![[
                Vec3::new(0.1, 0.1, 0.1),
                Vec3::new(1.2, 0.1, 0.1),
                Vec3::new(0.1, 1.2, 0.1),
            ]],
            material_ids: None,
        };
        let output = voxelize_surface_cpu(&mesh, &grid, &tiles, &VoxelizeOpts::default());
        let occupied = output.occupancy.iter().any(|word| *word != 0);
        assert!(occupied, "expected at least one occupied voxel");
    }
}
