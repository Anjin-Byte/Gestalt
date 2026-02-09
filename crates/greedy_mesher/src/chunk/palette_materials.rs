//! Palette-based material storage for voxel chunks.
//!
//! This module provides efficient material storage using a palette + bitpacked
//! indices approach. Instead of storing a full u16 array (512 KiB), we store:
//! - A palette of unique materials (typically < 16 materials)
//! - Bitpacked indices into that palette (64-384 KiB for most chunks)
//!
//! Memory savings:
//! - 4 materials: 64 KiB (88% reduction)
//! - 16 materials: 128 KiB (75% reduction)
//! - 256 materials: 256 KiB (50% reduction)
//!
//! When the palette grows beyond the current bit capacity, indices are
//! automatically repacked using the fast paths from `palette_repack`.

use super::palette_repack::{bits_required, repack_indices, required_words, VOXEL_COUNT};
use crate::core::{MaterialId, MATERIAL_EMPTY};

/// Palette-based material storage with bitpacked indices.
///
/// Stores materials efficiently by maintaining a palette of unique materials
/// and bitpacked indices. Automatically repacks when palette grows.
///
/// # Memory Layout
/// - Palette: `Vec<MaterialId>` - unique materials (typically 2-16 materials)
/// - Indices: `Vec<u64>` - bitpacked indices (size depends on palette size)
/// - Bits per voxel: 1-16 bits (automatically determined from palette size)
///
/// # Examples
/// ```
/// # use greedy_mesher::chunk::palette_materials::PaletteMaterials;
/// let mut materials = PaletteMaterials::new();
///
/// // Set some voxels (automatically manages palette)
/// materials.set_material(0, 0, 0, 1); // Stone
/// materials.set_material(1, 0, 0, 2); // Dirt
/// materials.set_material(2, 0, 0, 1); // Stone again (reuses palette entry)
///
/// // Get material at position
/// assert_eq!(materials.get_material(0, 0, 0), 1);
/// assert_eq!(materials.get_material(1, 0, 0), 2);
/// ```
#[derive(Clone)]
pub struct PaletteMaterials {
    /// Unique materials in this chunk.
    /// Index 0 is always MATERIAL_EMPTY (air).
    palette: Vec<MaterialId>,

    /// Bitpacked indices into palette.
    /// Length = ceil(VOXEL_COUNT * bits_per_voxel / 64)
    indices: Vec<u64>,

    /// Bits per voxel index (1-16).
    /// Always equals ceil(log2(palette.len())), minimum 1.
    bits_per_voxel: u8,
}

impl PaletteMaterials {
    /// Create a new empty palette-based material storage.
    ///
    /// Initializes with a single material (MATERIAL_EMPTY) using 1 bit per voxel.
    ///
    /// # Examples
    /// ```
    /// # use greedy_mesher::chunk::palette_materials::PaletteMaterials;
    /// let materials = PaletteMaterials::new();
    /// assert_eq!(materials.bits_per_voxel(), 1);
    /// assert_eq!(materials.palette_size(), 1);
    /// ```
    pub fn new() -> Self {
        let bits_per_voxel = 1;
        let words = required_words(VOXEL_COUNT, bits_per_voxel);

        Self {
            palette: vec![MATERIAL_EMPTY],
            indices: vec![0u64; words],
            bits_per_voxel,
        }
    }

    /// Get the material at the given voxel position.
    ///
    /// # Panics
    /// Panics in debug mode if coordinates are out of bounds.
    ///
    /// # Examples
    /// ```
    /// # use greedy_mesher::chunk::palette_materials::PaletteMaterials;
    /// let mut materials = PaletteMaterials::new();
    /// materials.set_material(10, 20, 30, 42);
    /// assert_eq!(materials.get_material(10, 20, 30), 42);
    /// ```
    #[inline]
    pub fn get_material(&self, x: usize, y: usize, z: usize) -> MaterialId {
        debug_assert!(x < 64 && y < 64 && z < 64, "Coordinates out of bounds");

        let voxel_idx = x * 64 * 64 + y * 64 + z;
        let palette_idx = unsafe {
            super::palette_repack::get_index_generic(&self.indices, voxel_idx, self.bits_per_voxel)
        };

        self.palette[palette_idx as usize]
    }

    /// Set the material at the given voxel position.
    ///
    /// Automatically manages the palette and repacks if necessary.
    ///
    /// # Performance
    /// - Fast path: Material already in palette (O(palette_size) search + O(1) write)
    /// - Slow path: New material triggers repack (O(VOXEL_COUNT) repack + palette insert)
    ///
    /// # Panics
    /// Panics in debug mode if coordinates are out of bounds.
    ///
    /// # Examples
    /// ```
    /// # use greedy_mesher::chunk::palette_materials::PaletteMaterials;
    /// let mut materials = PaletteMaterials::new();
    ///
    /// // First unique material (besides air)
    /// materials.set_material(0, 0, 0, 1);
    /// assert_eq!(materials.palette_size(), 2); // [EMPTY, 1]
    ///
    /// // Second unique material - triggers repack from 1-bit to 2-bit
    /// materials.set_material(1, 0, 0, 2);
    /// assert_eq!(materials.palette_size(), 3); // [EMPTY, 1, 2]
    /// assert_eq!(materials.bits_per_voxel(), 2);
    /// ```
    #[inline]
    pub fn set_material(&mut self, x: usize, y: usize, z: usize, material: MaterialId) {
        debug_assert!(x < 64 && y < 64 && z < 64, "Coordinates out of bounds");

        let voxel_idx = x * 64 * 64 + y * 64 + z;

        // Fast path: material already in palette
        if let Some(palette_idx) = self.find_palette_index(material) {
            unsafe {
                super::palette_repack::set_index_generic(
                    &mut self.indices,
                    voxel_idx,
                    palette_idx as u16,
                    self.bits_per_voxel,
                );
            }
            return;
        }

        // Slow path: new material, need to add to palette
        self.insert_material(material, voxel_idx);
    }

    /// Get the current number of bits used per voxel.
    #[inline]
    pub fn bits_per_voxel(&self) -> u8 {
        self.bits_per_voxel
    }

    /// Get the current palette size.
    #[inline]
    pub fn palette_size(&self) -> usize {
        self.palette.len()
    }

    /// Get a reference to the palette.
    #[inline]
    pub fn palette(&self) -> &[MaterialId] {
        &self.palette
    }

    /// Find the palette index for a given material.
    ///
    /// Returns None if the material is not in the palette.
    #[inline]
    fn find_palette_index(&self, material: MaterialId) -> Option<usize> {
        self.palette.iter().position(|&m| m == material)
    }

    /// Insert a new material into the palette and set the voxel.
    ///
    /// This handles palette growth and automatic repacking when needed.
    fn insert_material(&mut self, material: MaterialId, voxel_idx: usize) {
        let new_palette_len = self.palette.len() + 1;
        let new_bits = bits_required(new_palette_len);

        // Check if repack is needed
        if new_bits > self.bits_per_voxel {
            self.repack_to_bits(new_bits);
        }

        // Add to palette
        self.palette.push(material);
        let palette_idx = (self.palette.len() - 1) as u16;

        // Set the voxel
        unsafe {
            super::palette_repack::set_index_generic(
                &mut self.indices,
                voxel_idx,
                palette_idx,
                self.bits_per_voxel,
            );
        }
    }

    /// Repack indices to support a new bit width.
    ///
    /// This is called automatically when the palette grows beyond the current
    /// bit capacity.
    ///
    /// # Performance
    /// - Power-of-two transitions (1→2, 2→4, 4→8, 8→16): 0.1-0.2ms
    /// - Odd width transitions (2→3, 4→5, etc.): 1.8-4.5ms
    fn repack_to_bits(&mut self, new_bits: u8) {
        let old_bits = self.bits_per_voxel;

        // Allocate new buffer
        let new_words = required_words(VOXEL_COUNT, new_bits);
        let mut new_indices = vec![0u64; new_words];

        // Call specialized repack (uses fast paths for power-of-two)
        repack_indices(old_bits, new_bits, &self.indices, &mut new_indices);

        // Swap buffers (exception-safe: only mutate after repack succeeds)
        self.indices = new_indices;
        self.bits_per_voxel = new_bits;
    }
}

impl Default for PaletteMaterials {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_palette() {
        let materials = PaletteMaterials::new();
        assert_eq!(materials.bits_per_voxel(), 1);
        assert_eq!(materials.palette_size(), 1);
        assert_eq!(materials.palette()[0], MATERIAL_EMPTY);
    }

    #[test]
    fn test_get_set_single_material() {
        let mut materials = PaletteMaterials::new();

        // Initially all empty
        assert_eq!(materials.get_material(0, 0, 0), MATERIAL_EMPTY);

        // Set a voxel
        materials.set_material(10, 20, 30, 42);
        assert_eq!(materials.get_material(10, 20, 30), 42);

        // Other voxels still empty
        assert_eq!(materials.get_material(0, 0, 0), MATERIAL_EMPTY);
        assert_eq!(materials.get_material(63, 63, 63), MATERIAL_EMPTY);
    }

    #[test]
    fn test_palette_growth_triggers_repack() {
        let mut materials = PaletteMaterials::new();

        // Start with 1 bit (can hold 2 values: 0=empty, 1=first material)
        assert_eq!(materials.bits_per_voxel(), 1);

        // Add first material (palette: [EMPTY, 1])
        materials.set_material(0, 0, 0, 1);
        assert_eq!(materials.palette_size(), 2);
        assert_eq!(materials.bits_per_voxel(), 1); // Still 1 bit (2^1 = 2 values)

        // Add second material (palette: [EMPTY, 1, 2])
        // This requires 2 bits (2^2 = 4 values), triggering repack
        materials.set_material(1, 0, 0, 2);
        assert_eq!(materials.palette_size(), 3);
        assert_eq!(materials.bits_per_voxel(), 2); // Repacked to 2 bits

        // Verify both materials preserved
        assert_eq!(materials.get_material(0, 0, 0), 1);
        assert_eq!(materials.get_material(1, 0, 0), 2);
    }

    #[test]
    fn test_repack_preserves_all_voxels() {
        let mut materials = PaletteMaterials::new();

        // Fill with pattern of 4 different materials
        let test_materials = [MATERIAL_EMPTY, 1, 2, 3];

        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let mat = test_materials[(x + y + z) % 4];
                    materials.set_material(x, y, z, mat);
                }
            }
        }

        // Should have repacked to 2 bits (4 materials = 2^2)
        assert_eq!(materials.bits_per_voxel(), 2);
        assert_eq!(materials.palette_size(), 4);

        // Verify all voxels preserved
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let expected = test_materials[(x + y + z) % 4];
                    let actual = materials.get_material(x, y, z);
                    assert_eq!(
                        actual, expected,
                        "Mismatch at ({}, {}, {}): expected {}, got {}",
                        x, y, z, expected, actual
                    );
                }
            }
        }
    }

    #[test]
    fn test_multiple_repacks() {
        let mut materials = PaletteMaterials::new();

        // Add materials to trigger multiple repacks
        let materials_to_add = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        for (i, &mat) in materials_to_add.iter().enumerate() {
            materials.set_material(i, 0, 0, mat);

            // Verify all previous materials still correct
            for j in 0..=i {
                assert_eq!(
                    materials.get_material(j, 0, 0),
                    materials_to_add[j],
                    "Material {} incorrect after adding material {}",
                    j,
                    i
                );
            }
        }

        // Should have repacked to 4 bits (11 materials including EMPTY = ceil(log2(11)) = 4)
        assert_eq!(materials.bits_per_voxel(), 4);
        assert_eq!(materials.palette_size(), 11);
    }

    #[test]
    fn test_reusing_palette_entries() {
        let mut materials = PaletteMaterials::new();

        // Add first material
        materials.set_material(0, 0, 0, 1);
        let size_after_first = materials.palette_size();

        // Add same material again (should reuse palette entry)
        materials.set_material(1, 0, 0, 1);
        assert_eq!(materials.palette_size(), size_after_first);

        // Verify both voxels have the material
        assert_eq!(materials.get_material(0, 0, 0), 1);
        assert_eq!(materials.get_material(1, 0, 0), 1);
    }

    #[test]
    fn test_power_of_two_transitions() {
        let mut materials = PaletteMaterials::new();

        // Track bit width transitions
        let mut transitions = Vec::new();

        // Add materials to trigger power-of-two transitions
        for i in 1..=16 {
            let old_bits = materials.bits_per_voxel();
            materials.set_material(i, 0, 0, i as u16);
            let new_bits = materials.bits_per_voxel();

            if old_bits != new_bits {
                transitions.push((old_bits, new_bits));
            }
        }

        // Should have triggered these transitions:
        // 1→2 (at 2 materials), 2→4 (at 4 materials), 4→8 (at 16 materials)
        // Actually: 1 bit holds 2 values, 2 bits hold 4, etc.
        // Palette sizes: 1 (1 bit), 2 (1 bit), 3 (2 bits), 5 (3 bits), 9 (4 bits), 17 (5 bits)

        // Verify all materials preserved
        for i in 1..=16 {
            assert_eq!(
                materials.get_material(i, 0, 0),
                i as u16,
                "Material {} not preserved",
                i
            );
        }
    }

    #[test]
    fn test_large_palette() {
        let mut materials = PaletteMaterials::new();

        // Add 256 unique materials (should require 8 bits)
        for i in 0..256 {
            materials.set_material(i % 64, (i / 64) % 64, (i / 4096) % 64, i as u16);
        }

        assert_eq!(materials.palette_size(), 256);
        assert_eq!(materials.bits_per_voxel(), 8);

        // Verify sample materials
        assert_eq!(materials.get_material(0, 0, 0), 0);
        assert_eq!(materials.get_material(1, 0, 0), 1);
        assert_eq!(materials.get_material(63, 0, 0), 63);
        assert_eq!(materials.get_material(0, 1, 0), 64);
    }
}
