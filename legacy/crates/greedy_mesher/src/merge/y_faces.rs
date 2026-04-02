//! Greedy merge for +Y and -Y faces.
//!
//! Y-axis faces are horizontal planes. We sweep through Y slices,
//! and for each slice merge faces in the XZ plane.
//! Width extends along X, height extends along Z.

use crate::core::{BinaryChunk, FaceMasks, pack_quad, CS};

/// Greedy merge for Y-axis faces (+Y or -Y).
///
/// For each Y slice, we scan the XZ plane and greedily merge
/// adjacent faces with the same material into larger quads.
pub fn greedy_merge_y_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    output: &mut Vec<u64>,
) {
    // Work buffer: true if face at (x, z) has been processed
    let mut processed = [[false; CS]; CS];

    // Process each Y slice
    for y in 0..CS {
        // Reset processed flags for new slice
        for row in &mut processed {
            row.fill(false);
        }

        // Scan XZ plane
        for start_x in 0..CS {
            for start_z in 0..CS {
                // Skip if already processed
                if processed[start_x][start_z] {
                    continue;
                }

                let mask_x = start_x + 1;
                let mask_z = start_z + 1;
                let face_mask = masks.get(face, mask_x, mask_z);
                let is_visible = (face_mask >> y) & 1 != 0;

                if !is_visible {
                    continue;
                }

                let material = chunk.get_material(mask_x, y + 1, mask_z);

                // Extend width in +X direction
                let mut width = 1u32;
                while (start_x + width as usize) < CS {
                    let next_x = start_x + width as usize;
                    if processed[next_x][start_z] {
                        break;
                    }

                    let next_mask_x = next_x + 1;
                    let next_face_mask = masks.get(face, next_mask_x, mask_z);
                    let next_visible = (next_face_mask >> y) & 1 != 0;

                    if !next_visible {
                        break;
                    }

                    let next_material = chunk.get_material(next_mask_x, y + 1, mask_z);
                    if next_material != material {
                        break;
                    }

                    width += 1;
                }

                // Extend height in +Z direction
                let mut height = 1u32;
                'height_loop: while (start_z + height as usize) < CS {
                    let next_z = start_z + height as usize;
                    let next_mask_z = next_z + 1;

                    // Check that entire width can extend
                    for check_x in start_x..(start_x + width as usize) {
                        if processed[check_x][next_z] {
                            break 'height_loop;
                        }

                        let check_mask_x = check_x + 1;
                        let check_face_mask = masks.get(face, check_mask_x, next_mask_z);
                        let check_visible = (check_face_mask >> y) & 1 != 0;

                        if !check_visible {
                            break 'height_loop;
                        }

                        let check_material = chunk.get_material(check_mask_x, y + 1, next_mask_z);
                        if check_material != material {
                            break 'height_loop;
                        }
                    }

                    height += 1;
                }

                // Mark region as processed
                for px in start_x..(start_x + width as usize) {
                    for pz in start_z..(start_z + height as usize) {
                        processed[px][pz] = true;
                    }
                }

                // Emit the quad
                output.push(pack_quad(
                    start_x as u32,
                    y as u32,
                    start_z as u32,
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
    use crate::core::{FACE_POS_Y, unpack_quad};

    fn create_slab(y: usize, x_range: std::ops::Range<usize>, z_range: std::ops::Range<usize>) -> BinaryChunk {
        let mut chunk = BinaryChunk::new();
        for x in x_range {
            for z in z_range.clone() {
                chunk.set(x + 1, y + 1, z + 1, 1); // +1 for padding
            }
        }
        chunk
    }

    #[test]
    fn single_voxel_single_quad() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_y_faces(FACE_POS_Y, &chunk, &masks, &mut quads);

        assert_eq!(quads.len(), 1);
        let (x, y, z, w, h, mat) = unpack_quad(quads[0]);
        assert_eq!((x, y, z, w, h, mat), (31, 31, 31, 1, 1, 1));
    }

    #[test]
    fn slab_10x10_merges_to_one_quad() {
        // 10x10 slab at y=20
        let chunk = create_slab(20, 20..30, 20..30);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads_pos_y = Vec::new();
        greedy_merge_y_faces(FACE_POS_Y, &chunk, &masks, &mut quads_pos_y);

        // Should merge into single quad
        assert_eq!(quads_pos_y.len(), 1, "10x10 slab should merge to 1 quad");

        let (_x, _y, _z, w, h, _mat) = unpack_quad(quads_pos_y[0]);
        assert_eq!(w * h, 100, "Quad should cover 100 faces");
    }

    #[test]
    fn two_materials_two_quads() {
        let mut chunk = BinaryChunk::new();
        // Two adjacent voxels with different materials
        chunk.set(32, 32, 32, 1);
        chunk.set(33, 32, 32, 2);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_y_faces(FACE_POS_Y, &chunk, &masks, &mut quads);

        // Should not merge due to different materials
        assert_eq!(quads.len(), 2);
    }

    #[test]
    fn row_merges_in_x() {
        let mut chunk = BinaryChunk::new();
        // 5 voxels in a row along X
        for x in 20..25 {
            chunk.set(x, 32, 32, 1);
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_y_faces(FACE_POS_Y, &chunk, &masks, &mut quads);

        // Should merge into single quad with width 5
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 5);
        assert_eq!(h, 1);
    }

    #[test]
    fn column_merges_in_z() {
        let mut chunk = BinaryChunk::new();
        // 5 voxels in a column along Z
        for z in 20..25 {
            chunk.set(32, 32, z, 1);
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_y_faces(FACE_POS_Y, &chunk, &masks, &mut quads);

        // Should merge into single quad with height 5
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 1);
        assert_eq!(h, 5);
    }
}
