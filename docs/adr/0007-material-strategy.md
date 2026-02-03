# 0007 - Material Strategy

## Status
Proposed

## Context

The voxel mesh system requires a material strategy that supports:
1. **Per-voxel material assignment** - Each voxel can have a distinct material
2. **Texture support** - Materials can reference textures, not just solid colors
3. **Runtime modification** - Materials can be changed during editing sessions
4. **Efficient rendering** - Minimize draw calls and material switches

**Current State:**
- `voxel_types: [u8; CS_P3]` stores 8-bit material IDs (max 256 materials)
- `MeshOutput.colors: Option<Vec<f32>>` provides RGB vertex colors
- No UV coordinate generation
- No texture atlas infrastructure
- Single `MeshStandardMaterial` for all voxels

**Requirements:**
- Support 4096+ unique materials (for large texture atlases)
- Generate UV coordinates for textured rendering
- Material-aware greedy merging (only merge same-material faces)
- Tiling textures on merged quads (a 4x4 merged quad should tile 4x4)
- PBR material properties (roughness, metalness)
- Runtime material palette editing

**Alternatives Considered:**

1. **Vertex Colors Only** (current approach)
   - Pros: Simple, no texture management
   - Cons: No texture detail, limited visual quality

2. **Per-Face Texture Binding**
   - Pros: Maximum flexibility
   - Cons: Requires draw call per material, defeats greedy meshing benefits

3. **Texture Atlas with UV Generation**
   - Pros: Single draw call, texture detail, compatible with greedy meshing
   - Cons: Atlas management complexity, UV tiling math

4. **Array Texture with Material Index**
   - Pros: Clean material indexing, WebGPU native support
   - Cons: WebGL2 has limitations, requires shader modifications

## Decision

Adopt **Texture Atlas with UV Generation** using:
- 16-bit material IDs (`MaterialId = u16`) for 65536 material capacity
- 2D array texture atlas (WebGL2/WebGPU compatible)
- UV generation during quad expansion with proper tiling
- Material registry on TypeScript side for runtime management

### Material ID Format

```rust
/// Material identifier (16-bit for texture atlas indexing)
pub type MaterialId = u16;

/// Reserved material values
pub const MATERIAL_EMPTY: MaterialId = 0;
pub const MATERIAL_DEFAULT: MaterialId = 1;
```

### Material Definition

```rust
/// Material properties stored in registry (not per-voxel)
#[derive(Clone, Debug)]
pub struct MaterialDef {
    /// Base color (RGBA, used if no texture or as tint)
    pub color: [f32; 4],

    /// PBR properties
    pub roughness: f32,
    pub metalness: f32,
    pub emissive: [f32; 3],

    /// Texture atlas reference (None = solid color)
    pub texture: Option<TextureRef>,
}

/// Reference to texture in atlas
#[derive(Clone, Debug)]
pub struct TextureRef {
    /// Which atlas layer (for array textures)
    pub layer: u16,

    /// UV region in atlas (normalized 0-1)
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
}

impl Default for MaterialDef {
    fn default() -> Self {
        Self {
            color: [0.8, 0.8, 0.8, 1.0],
            roughness: 0.5,
            metalness: 0.0,
            emissive: [0.0, 0.0, 0.0],
            texture: None,
        }
    }
}
```

### Texture Atlas Specification

| Property | Value | Rationale |
|----------|-------|-----------|
| Tile size | 16x16 pixels | Good detail at typical voxel scales |
| Atlas dimensions | 256x256 per layer | 256 tiles per layer (16x16 grid) |
| Layer count | Up to 16 | 4096 total textures |
| Format | RGBA8 | Standard, widely supported |
| Filtering | Nearest + mipmap | Preserves voxel aesthetic |

**Atlas Layout:**
```
Layer 0: Materials 0-255
Layer 1: Materials 256-511
...
Layer 15: Materials 3840-4095

Within each layer (256x256):
┌────┬────┬────┬────┬─...─┬────┐
│ 0  │ 1  │ 2  │ 3  │     │ 15 │  Row 0
├────┼────┼────┼────┼─...─┼────┤
│ 16 │ 17 │ 18 │ 19 │     │ 31 │  Row 1
├────┼────┼────┼────┼─...─┼────┤
│... │... │... │... │     │... │
├────┼────┼────┼────┼─...─┼────┤
│240 │241 │242 │243 │     │255 │  Row 15
└────┴────┴────┴────┴─...─┴────┘
```

### Extended Mesh Output

```rust
/// Mesh output with UV and material support
#[derive(Default)]
pub struct MeshOutput {
    /// Vertex positions (3 floats per vertex)
    pub positions: Vec<f32>,

    /// Vertex normals (3 floats per vertex)
    pub normals: Vec<f32>,

    /// Triangle indices
    pub indices: Vec<u32>,

    /// UV coordinates (2 floats per vertex)
    /// Tiled appropriately for merged quads
    pub uvs: Vec<f32>,

    /// Per-vertex material ID (for shader lookup)
    /// All vertices of a quad share the same material
    pub material_ids: Vec<u16>,
}
```

### UV Generation Algorithm

During quad expansion, UVs are generated with tiling:

```rust
fn emit_quad_with_uvs(
    face: usize,
    x: u32, y: u32, z: u32,
    width: u32, height: u32,
    material: MaterialId,
    material_registry: &MaterialRegistry,
    voxel_size: f32,
    origin: [f32; 3],
    output: &mut MeshOutput,
) {
    let base_vertex = output.vertex_count() as u32;

    // Get material's texture region (or default 0-1 range)
    let (uv_min, uv_max) = match material_registry.get(material) {
        Some(mat) if mat.texture.is_some() => {
            let tex = mat.texture.as_ref().unwrap();
            (tex.uv_min, tex.uv_max)
        }
        _ => ([0.0, 0.0], [1.0, 1.0]),
    };

    let uv_width = uv_max[0] - uv_min[0];
    let uv_height = uv_max[1] - uv_min[1];

    // Tile UVs based on quad dimensions
    // A 4x3 merged quad tiles the texture 4x3 times
    let u_tiles = width as f32;
    let v_tiles = height as f32;

    // Four corners with tiled UVs
    let uvs: [[f32; 2]; 4] = [
        [uv_min[0], uv_min[1]],                                    // Bottom-left
        [uv_min[0] + uv_width * u_tiles, uv_min[1]],              // Bottom-right
        [uv_min[0] + uv_width * u_tiles, uv_min[1] + uv_height * v_tiles], // Top-right
        [uv_min[0], uv_min[1] + uv_height * v_tiles],             // Top-left
    ];

    // ... emit positions, normals as before ...

    // Emit UVs
    for uv in &uvs {
        output.uvs.extend_from_slice(uv);
    }

    // Emit material IDs (same for all 4 vertices)
    for _ in 0..4 {
        output.material_ids.push(material);
    }
}
```

### TypeScript Material Registry

```typescript
export interface MaterialDef {
    color: [number, number, number, number];  // RGBA
    roughness: number;
    metalness: number;
    emissive: [number, number, number];
    texture?: TextureRef;
}

export interface TextureRef {
    layer: number;
    uvMin: [number, number];
    uvMax: [number, number];
}

export interface MaterialRegistry {
    // Material management
    define(id: MaterialId, def: MaterialDef): void;
    get(id: MaterialId): MaterialDef | undefined;
    update(id: MaterialId, changes: Partial<MaterialDef>): void;
    delete(id: MaterialId): void;

    // Iteration
    entries(): IterableIterator<[MaterialId, MaterialDef]>;
    count(): number;

    // Atlas management
    loadTexture(id: MaterialId, url: string): Promise<void>;
    getAtlasTexture(): THREE.DataArrayTexture;

    // Change notification
    onChange(callback: (id: MaterialId) => void): () => void;
}
```

### Three.js Integration

```typescript
// Custom shader material for atlas lookup
const voxelMaterial = new THREE.ShaderMaterial({
    uniforms: {
        atlas: { value: materialRegistry.getAtlasTexture() },
        materialData: { value: materialRegistry.getDataTexture() },
    },
    vertexShader: `
        attribute float materialId;
        varying vec2 vUv;
        varying float vMaterialId;

        void main() {
            vUv = uv;
            vMaterialId = materialId;
            gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }
    `,
    fragmentShader: `
        uniform sampler2DArray atlas;
        uniform sampler2D materialData;
        varying vec2 vUv;
        varying float vMaterialId;

        void main() {
            // Look up material properties
            vec4 matProps = texture2D(materialData, vec2(vMaterialId / 4096.0, 0.5));

            // Calculate atlas layer and sample
            float layer = floor(vMaterialId / 256.0);
            vec3 atlasCoord = vec3(fract(vUv), layer);
            vec4 texColor = texture(atlas, atlasCoord);

            // Combine base color with texture
            gl_FragColor = texColor * matProps;
        }
    `,
});
```

### Voxel Data Format Update

```rust
/// Binary chunk with 16-bit materials
pub struct BinaryChunk {
    /// Opaque mask: one bit per voxel
    pub opaque_mask: [u64; CS_P2],

    /// Material IDs: 16-bit per voxel
    pub materials: [MaterialId; CS_P3],
}
```

### Runtime Material Editing

```typescript
// Example: Change grass material to dirt
materialRegistry.update(MATERIAL_GRASS, {
    color: [0.5, 0.3, 0.1, 1.0],
    texture: { layer: 0, uvMin: [0.0625, 0], uvMax: [0.125, 0.0625] }
});

// All chunks using MATERIAL_GRASS automatically update
// (shader reads from materialData texture)
```

### Data Flow

```
┌─────────────────┐
│ GPU Voxelizer   │
│ (owner_id)      │
└────────┬────────┘
         │ Material mapping
         ▼
┌─────────────────┐
│ BinaryChunk     │
│ (materials u16) │
└────────┬────────┘
         │ Greedy meshing
         ▼
┌─────────────────┐
│ MeshOutput      │
│ (uvs, mat_ids)  │
└────────┬────────┘
         │ WASM → JS
         ▼
┌─────────────────┐
│ BufferGeometry  │
│ + ShaderMaterial│
└────────┬────────┘
         │ Three.js render
         ▼
┌─────────────────┐
│ Textured Mesh   │
└─────────────────┘
```

## Consequences

### Positive
- **Rich visuals**: Full texture support with proper tiling
- **Single draw call**: Atlas approach maintains greedy meshing benefits
- **Flexible materials**: PBR properties, runtime modification
- **Future-proof**: 65536 material capacity

### Negative
- **Memory overhead**: 16-bit materials double voxel storage (256KB → 512KB per chunk)
- **Shader complexity**: Custom shader required instead of standard materials
- **Atlas management**: Texture loading, layout, mipmap generation complexity
- **UV precision**: Large merged quads may have UV precision issues at edges

### Constraints Introduced
- Materials must be pre-registered before use
- Texture tiles must be uniform size (16x16)
- Maximum 4096 textured materials (16 layers x 256 tiles)

## References
- [greedy-mesh-implementation-plan.md](../greedy-mesh-implementation-plan.md) - Core meshing algorithm
- [0003-binary-greedy-meshing.md](0003-binary-greedy-meshing.md) - Algorithm decision
- [Three.js DataArrayTexture](https://threejs.org/docs/#api/en/textures/DataArrayTexture)
