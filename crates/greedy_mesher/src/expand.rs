//! Quad expansion to vertex arrays.
//!
//! Converts packed quad data into standard vertex arrays (positions, normals, indices)
//! suitable for GPU rendering. Optionally generates UV coordinates and material IDs
//! for texture atlas rendering.

use crate::core::{
    MeshOutput, MaterialId, unpack_quad,
    FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z,
    FACE_NORMALS,
};

/// Expand packed quads into vertex arrays (positions, normals, indices only).
///
/// # Arguments
/// * `packed_quads` - Array of 6 quad vectors, one per face direction
/// * `voxel_size` - Size of each voxel in world units
/// * `origin` - World position offset for the chunk
pub fn expand_quads(
    packed_quads: &[Vec<u64>; 6],
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshOutput {
    let total_quads: usize = packed_quads.iter().map(|q| q.len()).sum();
    let mut output = MeshOutput {
        positions: Vec::with_capacity(total_quads * 4 * 3),
        normals: Vec::with_capacity(total_quads * 4 * 3),
        indices: Vec::with_capacity(total_quads * 6),
        uvs: Vec::new(),
        material_ids: Vec::new(),
    };

    for (face, quads) in packed_quads.iter().enumerate() {
        let normal = FACE_NORMALS[face];

        for &quad in quads {
            let (x, y, z, w, h, _material) = unpack_quad(quad);
            emit_quad_basic(
                face,
                x, y, z, w, h,
                &normal,
                voxel_size,
                origin,
                &mut output,
            );
        }
    }

    output
}

/// Expand packed quads with UV coordinates and material IDs.
///
/// UVs tile based on quad dimensions - a 4x3 quad will have UV range [0,4] x [0,3],
/// allowing the shader to use fract(uv) for seamless tiling.
///
/// # Arguments
/// * `packed_quads` - Array of 6 quad vectors, one per face direction
/// * `voxel_size` - Size of each voxel in world units
/// * `origin` - World position offset for the chunk
pub fn expand_quads_with_uvs(
    packed_quads: &[Vec<u64>; 6],
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshOutput {
    let total_quads: usize = packed_quads.iter().map(|q| q.len()).sum();
    let mut output = MeshOutput::with_capacity(total_quads);

    for (face, quads) in packed_quads.iter().enumerate() {
        let normal = FACE_NORMALS[face];

        for &quad in quads {
            let (x, y, z, w, h, material) = unpack_quad(quad);
            emit_quad_with_uvs(
                face,
                x, y, z, w, h,
                material,
                &normal,
                voxel_size,
                origin,
                &mut output,
            );
        }
    }

    output
}

/// Emit a single quad as 4 vertices and 6 indices (basic version, no UVs).
fn emit_quad_basic(
    face: usize,
    x: u32, y: u32, z: u32,
    width: u32, height: u32,
    normal: &[f32; 3],
    voxel_size: f32,
    origin: [f32; 3],
    output: &mut MeshOutput,
) {
    let base_vertex = output.positions.len() as u32 / 3;

    // World-space base position
    let bx = origin[0] + x as f32 * voxel_size;
    let by = origin[1] + y as f32 * voxel_size;
    let bz = origin[2] + z as f32 * voxel_size;

    // Width and height in world units
    let w = width as f32 * voxel_size;
    let h = height as f32 * voxel_size;

    // Generate 4 corners based on face direction
    let corners = compute_quad_corners(face, bx, by, bz, w, h, voxel_size);

    // Add vertices
    for corner in &corners {
        output.positions.extend_from_slice(corner);
        output.normals.extend_from_slice(normal);
    }

    // Add indices (two triangles, CCW winding)
    output.indices.extend_from_slice(&[
        base_vertex,
        base_vertex + 1,
        base_vertex + 2,
        base_vertex,
        base_vertex + 2,
        base_vertex + 3,
    ]);
}

/// Emit a single quad with UV coordinates and material ID.
fn emit_quad_with_uvs(
    face: usize,
    x: u32, y: u32, z: u32,
    width: u32, height: u32,
    material: MaterialId,
    normal: &[f32; 3],
    voxel_size: f32,
    origin: [f32; 3],
    output: &mut MeshOutput,
) {
    let base_vertex = output.positions.len() as u32 / 3;

    // World-space base position
    let bx = origin[0] + x as f32 * voxel_size;
    let by = origin[1] + y as f32 * voxel_size;
    let bz = origin[2] + z as f32 * voxel_size;

    // Width and height in world units
    let w = width as f32 * voxel_size;
    let h = height as f32 * voxel_size;

    // UV tiling: quad dimensions determine how many times texture repeats
    let u_tiles = width as f32;
    let v_tiles = height as f32;

    // Generate corners and UVs based on face direction
    let (corners, uvs) = compute_quad_corners_with_uvs(face, bx, by, bz, w, h, voxel_size, u_tiles, v_tiles);

    // Add vertices
    for i in 0..4 {
        output.positions.extend_from_slice(&corners[i]);
        output.normals.extend_from_slice(normal);
        output.uvs.extend_from_slice(&uvs[i]);
        output.material_ids.push(material);
    }

    // Add indices (two triangles, CCW winding)
    output.indices.extend_from_slice(&[
        base_vertex,
        base_vertex + 1,
        base_vertex + 2,
        base_vertex,
        base_vertex + 2,
        base_vertex + 3,
    ]);
}

/// Compute the 4 corner positions for a quad based on face direction.
fn compute_quad_corners(
    face: usize,
    bx: f32, by: f32, bz: f32,
    w: f32, h: f32,
    voxel_size: f32,
) -> [[f32; 3]; 4] {
    match face {
        FACE_POS_Y => [
            [bx, by + voxel_size, bz],
            [bx + w, by + voxel_size, bz],
            [bx + w, by + voxel_size, bz + h],
            [bx, by + voxel_size, bz + h],
        ],
        FACE_NEG_Y => [
            [bx, by, bz],
            [bx, by, bz + h],
            [bx + w, by, bz + h],
            [bx + w, by, bz],
        ],
        // X-faces: width extends along Y, height extends along Z
        FACE_POS_X => [
            [bx + voxel_size, by, bz],
            [bx + voxel_size, by + w, bz],
            [bx + voxel_size, by + w, bz + h],
            [bx + voxel_size, by, bz + h],
        ],
        FACE_NEG_X => [
            [bx, by, bz],
            [bx, by, bz + h],
            [bx, by + w, bz + h],
            [bx, by + w, bz],
        ],
        FACE_POS_Z => [
            [bx, by, bz + voxel_size],
            [bx + w, by, bz + voxel_size],
            [bx + w, by + h, bz + voxel_size],
            [bx, by + h, bz + voxel_size],
        ],
        FACE_NEG_Z => [
            [bx, by, bz],
            [bx, by + h, bz],
            [bx + w, by + h, bz],
            [bx + w, by, bz],
        ],
        _ => unreachable!("Invalid face direction"),
    }
}

/// Compute corner positions and UV coordinates for a quad.
fn compute_quad_corners_with_uvs(
    face: usize,
    bx: f32, by: f32, bz: f32,
    w: f32, h: f32,
    voxel_size: f32,
    u_tiles: f32, v_tiles: f32,
) -> ([[f32; 3]; 4], [[f32; 2]; 4]) {
    let corners = compute_quad_corners(face, bx, by, bz, w, h, voxel_size);

    // UV coordinates with tiling based on quad dimensions
    let uvs = match face {
        FACE_POS_Y => [
            [0.0, 0.0],
            [u_tiles, 0.0],
            [u_tiles, v_tiles],
            [0.0, v_tiles],
        ],
        FACE_NEG_Y => [
            [0.0, 0.0],
            [0.0, v_tiles],
            [u_tiles, v_tiles],
            [u_tiles, 0.0],
        ],
        FACE_POS_X => [
            [0.0, 0.0],
            [0.0, v_tiles],
            [u_tiles, v_tiles],
            [u_tiles, 0.0],
        ],
        FACE_NEG_X => [
            [0.0, 0.0],
            [u_tiles, 0.0],
            [u_tiles, v_tiles],
            [0.0, v_tiles],
        ],
        FACE_POS_Z => [
            [0.0, 0.0],
            [u_tiles, 0.0],
            [u_tiles, v_tiles],
            [0.0, v_tiles],
        ],
        FACE_NEG_Z => [
            [0.0, 0.0],
            [0.0, v_tiles],
            [u_tiles, v_tiles],
            [u_tiles, 0.0],
        ],
        _ => unreachable!("Invalid face direction"),
    };

    (corners, uvs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pack_quad;

    #[test]
    fn single_quad_vertex_count() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(10, 10, 10, 1, 1, 1));

        let output = expand_quads(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        assert_eq!(output.vertex_count(), 4);
        assert_eq!(output.triangle_count(), 2);
        assert_eq!(output.positions.len(), 12); // 4 vertices * 3 floats
        assert_eq!(output.normals.len(), 12);
        assert_eq!(output.indices.len(), 6); // 2 triangles * 3 indices
    }

    #[test]
    fn quad_with_uvs() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(10, 10, 10, 4, 3, 42));

        let output = expand_quads_with_uvs(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        assert_eq!(output.vertex_count(), 4);
        assert!(output.has_uvs());
        assert!(output.has_material_ids());
        assert_eq!(output.uvs.len(), 8); // 4 vertices * 2 floats
        assert_eq!(output.material_ids.len(), 4);

        // Check that all material IDs are correct
        for &mat in &output.material_ids {
            assert_eq!(mat, 42);
        }

        // Check UV tiling for 4x3 quad
        // Should have UVs from 0 to 4 in U and 0 to 3 in V
        let max_u = output.uvs.iter().step_by(2).cloned().fold(0.0f32, f32::max);
        let max_v = output.uvs.iter().skip(1).step_by(2).cloned().fold(0.0f32, f32::max);
        assert_eq!(max_u, 4.0);
        assert_eq!(max_v, 3.0);
    }

    #[test]
    fn multiple_quads() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(10, 10, 10, 1, 1, 1));
        packed_quads[FACE_NEG_Y].push(pack_quad(10, 10, 10, 1, 1, 1));
        packed_quads[FACE_POS_X].push(pack_quad(10, 10, 10, 1, 1, 1));

        let output = expand_quads(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        assert_eq!(output.vertex_count(), 12); // 3 quads * 4 vertices
        assert_eq!(output.triangle_count(), 6); // 3 quads * 2 triangles
    }

    #[test]
    fn empty_quads() {
        let packed_quads: [Vec<u64>; 6] = Default::default();

        let output = expand_quads(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        assert_eq!(output.vertex_count(), 0);
        assert_eq!(output.triangle_count(), 0);
    }

    #[test]
    fn voxel_size_scaling() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(0, 0, 0, 1, 1, 1));

        let output = expand_quads(&packed_quads, 0.5, [0.0, 0.0, 0.0]);

        // With voxel_size=0.5, a 1x1 quad should span 0.5 world units
        // Check that all positions are within expected range
        for &pos in &output.positions {
            assert!(pos >= 0.0 && pos <= 0.5);
        }
    }

    #[test]
    fn origin_offset() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(0, 0, 0, 1, 1, 1));

        let output = expand_quads(&packed_quads, 1.0, [10.0, 20.0, 30.0]);

        // All positions should be offset by origin
        let min_x = output.positions.iter().step_by(3).cloned().fold(f32::MAX, f32::min);
        let min_y = output.positions.iter().skip(1).step_by(3).cloned().fold(f32::MAX, f32::min);
        let min_z = output.positions.iter().skip(2).step_by(3).cloned().fold(f32::MAX, f32::min);

        assert!(min_x >= 10.0);
        assert!(min_y >= 20.0);
        assert!(min_z >= 30.0);
    }

    #[test]
    fn normal_directions() {
        let test_cases = [
            (FACE_POS_Y, [0.0, 1.0, 0.0]),
            (FACE_NEG_Y, [0.0, -1.0, 0.0]),
            (FACE_POS_X, [1.0, 0.0, 0.0]),
            (FACE_NEG_X, [-1.0, 0.0, 0.0]),
            (FACE_POS_Z, [0.0, 0.0, 1.0]),
            (FACE_NEG_Z, [0.0, 0.0, -1.0]),
        ];

        for (face, expected_normal) in test_cases {
            let mut packed_quads: [Vec<u64>; 6] = Default::default();
            packed_quads[face].push(pack_quad(0, 0, 0, 1, 1, 1));

            let output = expand_quads(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

            // Check all 4 vertices have correct normal
            for i in 0..4 {
                let nx = output.normals[i * 3];
                let ny = output.normals[i * 3 + 1];
                let nz = output.normals[i * 3 + 2];
                assert_eq!([nx, ny, nz], expected_normal, "Face {} has wrong normal", face);
            }
        }
    }
}
