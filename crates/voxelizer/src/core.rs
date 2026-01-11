use glam::{Mat4, Vec3};

#[derive(Debug, Clone)]
pub struct VoxelGridSpec {
    pub origin_world: Vec3,
    pub voxel_size: f32,
    pub dims: [u32; 3],
    pub world_to_grid: Option<Mat4>,
}

impl VoxelGridSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.voxel_size.is_finite() || self.voxel_size <= 0.0 {
            return Err("voxel_size must be finite and > 0".into());
        }
        if self.dims.iter().any(|&d| d == 0) {
            return Err("dims must be >= 1".into());
        }
        let total = self.dims[0] as u64 * self.dims[1] as u64 * self.dims[2] as u64;
        if total == 0 {
            return Err("dims product must be > 0".into());
        }
        if !self.origin_world.is_finite() {
            return Err("origin_world must be finite".into());
        }
        if let Some(mat) = self.world_to_grid {
            if !mat.is_finite() {
                return Err("world_to_grid must be finite".into());
            }
        }
        Ok(())
    }

    pub fn world_to_grid_matrix(&self) -> Mat4 {
        if let Some(mat) = self.world_to_grid {
            return mat;
        }
        let inv = 1.0 / self.voxel_size;
        Mat4::from_scale(Vec3::splat(inv)) * Mat4::from_translation(-self.origin_world)
    }

    pub fn num_voxels(&self) -> u64 {
        self.dims[0] as u64 * self.dims[1] as u64 * self.dims[2] as u64
    }
}

#[derive(Debug, Clone)]
pub struct TileSpec {
    pub tile_dims: [u32; 3],
    pub num_tiles: [u32; 3],
}

impl TileSpec {
    pub fn new(tile_dims: [u32; 3], grid_dims: [u32; 3]) -> Result<Self, String> {
        if tile_dims.iter().any(|&d| d == 0) {
            return Err("tile_dims must be >= 1".into());
        }
        let num_tiles = [
            (grid_dims[0] + tile_dims[0] - 1) / tile_dims[0],
            (grid_dims[1] + tile_dims[1] - 1) / tile_dims[1],
            (grid_dims[2] + tile_dims[2] - 1) / tile_dims[2],
        ];
        Ok(Self { tile_dims, num_tiles })
    }

    pub fn num_tiles_total(&self) -> u32 {
        self.num_tiles[0] * self.num_tiles[1] * self.num_tiles[2]
    }

    pub fn validate(&self, max_invocations: u32) -> Result<(), String> {
        if self.tile_dims.iter().any(|&d| d == 0) {
            return Err("tile_dims must be >= 1".into());
        }
        let tile_voxels = self.tile_dims[0] * self.tile_dims[1] * self.tile_dims[2];
        if tile_voxels > max_invocations {
            return Err(format!(
                "tile_dims product must be <= {} (got {})",
                max_invocations, tile_voxels
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MeshInput {
    pub triangles: Vec<[Vec3; 3]>,
    pub material_ids: Option<Vec<u32>>,
}

impl MeshInput {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ids) = &self.material_ids {
            if ids.len() != self.triangles.len() {
                return Err("material_ids length must match triangles length".into());
            }
        }
        for tri in &self.triangles {
            for v in tri.iter() {
                if !v.is_finite() {
                    return Err("triangle contains non-finite vertex".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct VoxelizeOpts {
  pub epsilon: f32,
  pub store_owner: bool,
  pub store_color: bool,
}

impl Default for VoxelizeOpts {
    fn default() -> Self {
        Self {
            epsilon: 1e-4,
            store_owner: true,
            store_color: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DispatchStats {
    pub triangles: u32,
    pub tiles: u32,
    pub voxels: u32,
    pub gpu_time_ms: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct VoxelizationOutput {
  pub occupancy: Vec<u32>,
  pub owner_id: Option<Vec<u32>>,
  pub color_rgba: Option<Vec<u32>>,
  pub stats: DispatchStats,
}

#[derive(Debug, Clone)]
pub struct SparseVoxelizationOutput {
    pub brick_dim: u32,
    pub brick_origins: Vec<[u32; 3]>,
    pub occupancy: Vec<u32>,
    pub owner_id: Option<Vec<u32>>,
    pub color_rgba: Option<Vec<u32>>,
    pub debug_flags: [u32; 3],
    pub debug_workgroups: u32,
    pub debug_tested: u32,
    pub debug_hits: u32,
    pub stats: DispatchStats,
}
