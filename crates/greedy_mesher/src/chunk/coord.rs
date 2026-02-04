//! Chunk coordinate type for chunk-space addressing.

use crate::core::CS;

/// Chunk coordinate in chunk-space (not world-space).
///
/// Each chunk represents a 64³ region of voxels (62³ usable with padding).
/// Coordinates can be negative to support unbounded worlds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    /// Origin chunk at (0, 0, 0).
    pub const ZERO: ChunkCoord = ChunkCoord { x: 0, y: 0, z: 0 };

    /// Create a new chunk coordinate.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Get the 6 face-adjacent neighbors.
    ///
    /// Returns neighbors in order: +X, -X, +Y, -Y, +Z, -Z
    pub fn neighbors(&self) -> [ChunkCoord; 6] {
        [
            ChunkCoord { x: self.x + 1, y: self.y, z: self.z },
            ChunkCoord { x: self.x - 1, y: self.y, z: self.z },
            ChunkCoord { x: self.x, y: self.y + 1, z: self.z },
            ChunkCoord { x: self.x, y: self.y - 1, z: self.z },
            ChunkCoord { x: self.x, y: self.y, z: self.z + 1 },
            ChunkCoord { x: self.x, y: self.y, z: self.z - 1 },
        ]
    }

    /// Convert world position to chunk coordinate.
    ///
    /// # Arguments
    /// * `world_pos` - World-space position [x, y, z]
    /// * `voxel_size` - Size of a single voxel in world units
    ///
    /// # Example
    /// ```
    /// use greedy_mesher::chunk::ChunkCoord;
    ///
    /// // With voxel_size=1.0, chunk (0,0,0) covers voxels [0..62)
    /// let coord = ChunkCoord::from_world([31.0, 31.0, 31.0], 1.0);
    /// assert_eq!(coord, ChunkCoord::new(0, 0, 0));
    ///
    /// // Position 62.0 is in chunk 1
    /// let coord = ChunkCoord::from_world([62.0, 0.0, 0.0], 1.0);
    /// assert_eq!(coord, ChunkCoord::new(1, 0, 0));
    /// ```
    pub fn from_world(world_pos: [f32; 3], voxel_size: f32) -> Self {
        let chunk_world_size = CS as f32 * voxel_size;
        ChunkCoord {
            x: (world_pos[0] / chunk_world_size).floor() as i32,
            y: (world_pos[1] / chunk_world_size).floor() as i32,
            z: (world_pos[2] / chunk_world_size).floor() as i32,
        }
    }

    /// Convert voxel index to chunk coordinate.
    ///
    /// Uses Euclidean division for correct negative coordinate handling.
    ///
    /// # Example
    /// ```
    /// use greedy_mesher::chunk::ChunkCoord;
    ///
    /// // Voxel at (100, 0, 0) is in chunk (1, 0, 0) since 100 / 62 = 1
    /// let coord = ChunkCoord::from_voxel([100, 0, 0]);
    /// assert_eq!(coord, ChunkCoord::new(1, 0, 0));
    ///
    /// // Negative voxels correctly map to negative chunks
    /// let coord = ChunkCoord::from_voxel([-1, 0, 0]);
    /// assert_eq!(coord, ChunkCoord::new(-1, 0, 0));
    /// ```
    pub fn from_voxel(voxel: [i32; 3]) -> Self {
        let cs = CS as i32;
        ChunkCoord {
            x: voxel[0].div_euclid(cs),
            y: voxel[1].div_euclid(cs),
            z: voxel[2].div_euclid(cs),
        }
    }

    /// Convert a world position to local voxel coordinates within this chunk.
    ///
    /// Returns coordinates in range [0, CS) for each axis.
    pub fn world_to_local(&self, world_pos: [f32; 3], voxel_size: f32) -> [u32; 3] {
        let chunk_world_size = CS as f32 * voxel_size;
        let local_x = ((world_pos[0] - self.x as f32 * chunk_world_size) / voxel_size) as u32;
        let local_y = ((world_pos[1] - self.y as f32 * chunk_world_size) / voxel_size) as u32;
        let local_z = ((world_pos[2] - self.z as f32 * chunk_world_size) / voxel_size) as u32;
        [
            local_x.min(CS as u32 - 1),
            local_y.min(CS as u32 - 1),
            local_z.min(CS as u32 - 1),
        ]
    }

    /// Convert a voxel index to local coordinates within this chunk.
    ///
    /// Uses Euclidean remainder for correct negative coordinate handling.
    pub fn voxel_to_local(voxel: [i32; 3]) -> [u32; 3] {
        let cs = CS as i32;
        [
            voxel[0].rem_euclid(cs) as u32,
            voxel[1].rem_euclid(cs) as u32,
            voxel[2].rem_euclid(cs) as u32,
        ]
    }

    /// Get world-space center of this chunk.
    pub fn center_world(&self, voxel_size: f32) -> [f32; 3] {
        let chunk_world_size = CS as f32 * voxel_size;
        let half_size = chunk_world_size * 0.5;
        [
            self.x as f32 * chunk_world_size + half_size,
            self.y as f32 * chunk_world_size + half_size,
            self.z as f32 * chunk_world_size + half_size,
        ]
    }

    /// Get world-space origin (minimum corner) of this chunk.
    pub fn origin_world(&self, voxel_size: f32) -> [f32; 3] {
        let chunk_world_size = CS as f32 * voxel_size;
        [
            self.x as f32 * chunk_world_size,
            self.y as f32 * chunk_world_size,
            self.z as f32 * chunk_world_size,
        ]
    }

    /// Calculate squared distance to a world position.
    ///
    /// Uses chunk center for distance calculation.
    pub fn distance_squared_to(&self, world_pos: [f32; 3], voxel_size: f32) -> f32 {
        let center = self.center_world(voxel_size);
        let dx = center[0] - world_pos[0];
        let dy = center[1] - world_pos[1];
        let dz = center[2] - world_pos[2];
        dx * dx + dy * dy + dz * dz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_constant() {
        assert_eq!(ChunkCoord::ZERO, ChunkCoord::new(0, 0, 0));
    }

    #[test]
    fn neighbors_returns_six() {
        let coord = ChunkCoord::new(5, 10, 15);
        let neighbors = coord.neighbors();

        assert_eq!(neighbors.len(), 6);
        assert_eq!(neighbors[0], ChunkCoord::new(6, 10, 15));  // +X
        assert_eq!(neighbors[1], ChunkCoord::new(4, 10, 15));  // -X
        assert_eq!(neighbors[2], ChunkCoord::new(5, 11, 15));  // +Y
        assert_eq!(neighbors[3], ChunkCoord::new(5, 9, 15));   // -Y
        assert_eq!(neighbors[4], ChunkCoord::new(5, 10, 16));  // +Z
        assert_eq!(neighbors[5], ChunkCoord::new(5, 10, 14));  // -Z
    }

    #[test]
    fn from_world_positive() {
        // Position at origin should be chunk 0
        let coord = ChunkCoord::from_world([0.0, 0.0, 0.0], 1.0);
        assert_eq!(coord, ChunkCoord::ZERO);

        // Position just inside first chunk
        let coord = ChunkCoord::from_world([31.0, 31.0, 31.0], 1.0);
        assert_eq!(coord, ChunkCoord::ZERO);

        // Position at start of second chunk
        let coord = ChunkCoord::from_world([62.0, 0.0, 0.0], 1.0);
        assert_eq!(coord, ChunkCoord::new(1, 0, 0));
    }

    #[test]
    fn from_world_negative() {
        // Just before origin goes to chunk -1
        let coord = ChunkCoord::from_world([-0.1, 0.0, 0.0], 1.0);
        assert_eq!(coord, ChunkCoord::new(-1, 0, 0));

        // Well into negative space
        let coord = ChunkCoord::from_world([-100.0, -100.0, -100.0], 1.0);
        assert_eq!(coord, ChunkCoord::new(-2, -2, -2));
    }

    #[test]
    fn from_voxel_positive() {
        let coord = ChunkCoord::from_voxel([0, 0, 0]);
        assert_eq!(coord, ChunkCoord::ZERO);

        let coord = ChunkCoord::from_voxel([61, 61, 61]);
        assert_eq!(coord, ChunkCoord::ZERO);

        let coord = ChunkCoord::from_voxel([62, 0, 0]);
        assert_eq!(coord, ChunkCoord::new(1, 0, 0));
    }

    #[test]
    fn from_voxel_negative() {
        // Euclidean division: -1 / 62 = -1 (not 0)
        let coord = ChunkCoord::from_voxel([-1, 0, 0]);
        assert_eq!(coord, ChunkCoord::new(-1, 0, 0));

        let coord = ChunkCoord::from_voxel([-62, 0, 0]);
        assert_eq!(coord, ChunkCoord::new(-1, 0, 0));

        let coord = ChunkCoord::from_voxel([-63, 0, 0]);
        assert_eq!(coord, ChunkCoord::new(-2, 0, 0));
    }

    #[test]
    fn voxel_to_local() {
        // Positive voxels
        assert_eq!(ChunkCoord::voxel_to_local([0, 0, 0]), [0, 0, 0]);
        assert_eq!(ChunkCoord::voxel_to_local([61, 61, 61]), [61, 61, 61]);
        assert_eq!(ChunkCoord::voxel_to_local([62, 0, 0]), [0, 0, 0]);
        assert_eq!(ChunkCoord::voxel_to_local([63, 1, 2]), [1, 1, 2]);

        // Negative voxels use Euclidean remainder
        assert_eq!(ChunkCoord::voxel_to_local([-1, 0, 0]), [61, 0, 0]);
        assert_eq!(ChunkCoord::voxel_to_local([-62, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn center_world() {
        let coord = ChunkCoord::ZERO;
        let center = coord.center_world(1.0);
        assert_eq!(center, [31.0, 31.0, 31.0]);

        let coord = ChunkCoord::new(1, 0, 0);
        let center = coord.center_world(1.0);
        assert_eq!(center, [93.0, 31.0, 31.0]); // 62 + 31 = 93
    }

    #[test]
    fn distance_squared_to() {
        let coord = ChunkCoord::ZERO;
        let center = coord.center_world(1.0);

        // Distance to own center is 0
        let dist = coord.distance_squared_to(center, 1.0);
        assert!(dist < 0.001);

        // Distance to origin
        let dist = coord.distance_squared_to([0.0, 0.0, 0.0], 1.0);
        let expected = 31.0 * 31.0 * 3.0;
        assert!((dist - expected).abs() < 0.001);
    }
}
