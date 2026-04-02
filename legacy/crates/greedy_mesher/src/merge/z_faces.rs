//! Greedy merge for +Z and -Z faces.
//!
//! Z-axis faces are vertical planes perpendicular to Z.
//! We sweep through Z slices, and for each slice merge faces in the XY plane.
//! Width extends along X, height extends along Y.

use crate::core::{BinaryChunk, FaceMasks, pack_quad, CS};

/// Greedy merge for Z-axis faces (+Z or -Z).
///
/// For each Z slice, we scan the XY plane and greedily merge
/// adjacent faces with the same material into larger quads.
pub fn greedy_merge_z_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    output: &mut Vec<u64>,
) {
    // Work buffer: true if face at (x, y) has been processed
    let mut processed = [[false; CS]; CS];

    // Process each Z slice
    for z in 0..CS {
        let mask_z = z + 1;

        // Reset processed flags for new slice
        for row in &mut processed {
            row.fill(false);
        }

        // Scan XY plane
        for start_x in 0..CS {
            for start_y in 0..CS {
                // Skip if already processed
                if processed[start_x][start_y] {
                    continue;
                }

                let mask_x = start_x + 1;
                let face_mask = masks.get(face, mask_x, mask_z);
                let is_visible = (face_mask >> start_y) & 1 != 0;

                if !is_visible {
                    continue;
                }

                let material = chunk.get_material(mask_x, start_y + 1, mask_z);

                // Extend width in +X direction
                let mut width = 1u32;
                while (start_x + width as usize) < CS {
                    let next_x = start_x + width as usize;
                    if processed[next_x][start_y] {
                        break;
                    }

                    let next_mask_x = next_x + 1;
                    let next_face_mask = masks.get(face, next_mask_x, mask_z);
                    let next_visible = (next_face_mask >> start_y) & 1 != 0;

                    if !next_visible {
                        break;
                    }

                    let next_material = chunk.get_material(next_mask_x, start_y + 1, mask_z);
                    if next_material != material {
                        break;
                    }

                    width += 1;
                }

                // Extend height in +Y direction
                let mut height = 1u32;
                'height_loop: while (start_y + height as usize) < CS {
                    let next_y = start_y + height as usize;

                    // Check that entire width can extend
                    for check_x in start_x..(start_x + width as usize) {
                        if processed[check_x][next_y] {
                            break 'height_loop;
                        }

                        let check_mask_x = check_x + 1;
                        let check_face_mask = masks.get(face, check_mask_x, mask_z);
                        let check_visible = (check_face_mask >> next_y) & 1 != 0;

                        if !check_visible {
                            break 'height_loop;
                        }

                        let check_material = chunk.get_material(check_mask_x, next_y + 1, mask_z);
                        if check_material != material {
                            break 'height_loop;
                        }
                    }

                    height += 1;
                }

                // Mark region as processed
                for px in start_x..(start_x + width as usize) {
                    for py in start_y..(start_y + height as usize) {
                        processed[px][py] = true;
                    }
                }

                // Emit the quad
                output.push(pack_quad(
                    start_x as u32,
                    start_y as u32,
                    z as u32,
                    width,
                    height,
                    material,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cull::cull_faces;
    use crate::core::{FACE_POS_Z, unpack_quad};

    #[test]
    fn single_voxel_single_quad() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_z_faces(FACE_POS_Z, &chunk, &masks, &mut quads);

        assert_eq!(quads.len(), 1);
    }

    #[test]
    fn row_x_merges() {
        let mut chunk = BinaryChunk::new();
        // 5 voxels in a row along X
        for x in 20..25 {
            chunk.set(x, 32, 32, 1);
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_z_faces(FACE_POS_Z, &chunk, &masks, &mut quads);

        // Should merge into single quad
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 5);
        assert_eq!(h, 1);
    }

    #[test]
    fn column_y_merges() {
        let mut chunk = BinaryChunk::new();
        // 5 voxels in a column along Y
        for y in 20..25 {
            chunk.set(32, y, 32, 1);
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_z_faces(FACE_POS_Z, &chunk, &masks, &mut quads);

        // Should merge into single quad
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 1);
        assert_eq!(h, 5);
    }

    #[test]
    fn slab_xy_merges() {
        let mut chunk = BinaryChunk::new();
        // 4x4 slab in XY at z=32
        for x in 20..24 {
            for y in 20..24 {
                chunk.set(x, y, 32, 1);
            }
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_z_faces(FACE_POS_Z, &chunk, &masks, &mut quads);

        // Should merge into single quad
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w * h, 16, "4x4 slab should merge to 16-face quad");
    }

    #[test]
    fn different_materials_separate() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);
        chunk.set(33, 32, 32, 2); // Different material

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_z_faces(FACE_POS_Z, &chunk, &masks, &mut quads);

        // Should not merge due to different materials
        assert_eq!(quads.len(), 2);
    }
}
