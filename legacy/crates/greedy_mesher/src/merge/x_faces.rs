//! Greedy merge for +X and -X faces.
//!
//! X-axis faces are vertical planes perpendicular to X.
//! We sweep through X slices, and for each slice merge faces in the YZ plane.
//! Width extends along Y, height extends along Z.

use crate::core::{BinaryChunk, FaceMasks, pack_quad, CS};

/// Greedy merge for X-axis faces (+X or -X).
///
/// For each X slice, we scan the YZ plane and greedily merge
/// adjacent faces with the same material into larger quads.
pub fn greedy_merge_x_faces(
    face: usize,
    chunk: &BinaryChunk,
    masks: &FaceMasks,
    output: &mut Vec<u64>,
) {
    // Work buffer: true if face at (y, z) has been processed
    let mut processed = [[false; CS]; CS];

    // Process each X slice
    for x in 0..CS {
        let mask_x = x + 1;

        // Reset processed flags for new slice
        for row in &mut processed {
            row.fill(false);
        }

        // Scan YZ plane
        for start_y in 0..CS {
            for start_z in 0..CS {
                // Skip if already processed
                if processed[start_y][start_z] {
                    continue;
                }

                let mask_z = start_z + 1;
                let face_mask = masks.get(face, mask_x, mask_z);
                let is_visible = (face_mask >> start_y) & 1 != 0;

                if !is_visible {
                    continue;
                }

                let material = chunk.get_material(mask_x, start_y + 1, mask_z);

                // Extend width in +Y direction
                let mut width = 1u32;
                while (start_y + width as usize) < CS {
                    let next_y = start_y + width as usize;
                    if processed[next_y][start_z] {
                        break;
                    }

                    let next_visible = (face_mask >> next_y) & 1 != 0;
                    if !next_visible {
                        break;
                    }

                    let next_material = chunk.get_material(mask_x, next_y + 1, mask_z);
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
                    let next_face_mask = masks.get(face, mask_x, next_mask_z);

                    // Check that entire width can extend
                    for check_y in start_y..(start_y + width as usize) {
                        if processed[check_y][next_z] {
                            break 'height_loop;
                        }

                        let check_visible = (next_face_mask >> check_y) & 1 != 0;
                        if !check_visible {
                            break 'height_loop;
                        }

                        let check_material = chunk.get_material(mask_x, check_y + 1, next_mask_z);
                        if check_material != material {
                            break 'height_loop;
                        }
                    }

                    height += 1;
                }

                // Mark region as processed
                for py in start_y..(start_y + width as usize) {
                    for pz in start_z..(start_z + height as usize) {
                        processed[py][pz] = true;
                    }
                }

                // Emit the quad
                output.push(pack_quad(
                    x as u32,
                    start_y as u32,
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
    use crate::core::{FACE_POS_X, unpack_quad};

    #[test]
    fn single_voxel_single_quad() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_x_faces(FACE_POS_X, &chunk, &masks, &mut quads);

        assert_eq!(quads.len(), 1);
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
        greedy_merge_x_faces(FACE_POS_X, &chunk, &masks, &mut quads);

        // Should merge into single quad
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 5);
        assert_eq!(h, 1);
    }

    #[test]
    fn column_z_merges() {
        let mut chunk = BinaryChunk::new();
        // 5 voxels in a row along Z
        for z in 20..25 {
            chunk.set(32, 32, z, 1);
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let mut quads = Vec::new();
        greedy_merge_x_faces(FACE_POS_X, &chunk, &masks, &mut quads);

        // Should merge into single quad
        assert_eq!(quads.len(), 1);
        let (_, _, _, w, h, _) = unpack_quad(quads[0]);
        assert_eq!(w, 1);
        assert_eq!(h, 5);
    }
}
