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
import {
  DataArrayTexture,
  DataTexture,
  RGBAFormat,
  UnsignedByteType,
  FloatType,
  NearestFilter,
  LinearMipMapLinearFilter,
  RepeatWrapping,
  ClampToEdgeWrapping,
} from 'three';

// Branded type for type safety (see typescript-architecture.md)
export type MaterialId = number & { readonly __brand: 'MaterialId' };

export function materialId(id: number): MaterialId {
  if (id < 0 || id > 65535 || !Number.isInteger(id)) {
    throw new Error(`Invalid material ID: ${id}`);
  }
  return id as MaterialId;
}

export const MATERIAL_EMPTY = materialId(0);
export const MATERIAL_DEFAULT = materialId(1);

export interface MaterialDef {
  color: [number, number, number, number];  // RGBA normalized 0-1
  roughness: number;                         // 0-1
  metalness: number;                         // 0-1
  emissive: [number, number, number];        // RGB normalized 0-1
  texture?: TextureRef;
}

export interface TextureRef {
  layer: number;              // Atlas layer (0-15)
  uvMin: [number, number];    // Normalized UV start
  uvMax: [number, number];    // Normalized UV end
}

/** Configuration for MaterialRegistry */
export interface MaterialRegistryConfig {
  /** Tile size in pixels (default: 16) */
  tileSize?: number;
  /** Tiles per row/column per layer (default: 16) */
  tilesPerAxis?: number;
  /** Maximum atlas layers (default: 16) */
  maxLayers?: number;
}

/** Default material properties */
const DEFAULT_MATERIAL: MaterialDef = {
  color: [0.8, 0.8, 0.8, 1.0],
  roughness: 0.5,
  metalness: 0.0,
  emissive: [0.0, 0.0, 0.0],
};
```

### MaterialRegistry Implementation

```typescript
/**
 * Manages material definitions and texture atlas for voxel rendering.
 *
 * Usage:
 * ```typescript
 * const registry = new MaterialRegistry();
 * registry.define(materialId(1), { color: [1, 0, 0, 1], roughness: 0.3 });
 * await registry.loadTexture(materialId(1), '/textures/brick.png');
 * const material = createVoxelMaterial(registry.getAtlasTexture(), registry.getDataTexture());
 * ```
 */
export class MaterialRegistry {
  private materials: Map<MaterialId, MaterialDef> = new Map();
  private changeCallbacks: Set<(id: MaterialId) => void> = new Set();

  // Atlas configuration
  private readonly tileSize: number;
  private readonly tilesPerAxis: number;
  private readonly tilesPerLayer: number;
  private readonly maxLayers: number;
  private readonly atlasSize: number;

  // GPU textures (lazily created)
  private atlasTexture: DataArrayTexture | null = null;
  private dataTexture: DataTexture | null = null;
  private atlasData: Uint8Array;
  private materialDataArray: Float32Array;
  private dirty: Set<MaterialId> = new Set();
  private atlasDirty = false;
  private dataDirty = false;

  constructor(config: MaterialRegistryConfig = {}) {
    this.tileSize = config.tileSize ?? 16;
    this.tilesPerAxis = config.tilesPerAxis ?? 16;
    this.tilesPerLayer = this.tilesPerAxis * this.tilesPerAxis; // 256
    this.maxLayers = config.maxLayers ?? 16;
    this.atlasSize = this.tileSize * this.tilesPerAxis; // 256

    // Pre-allocate atlas data (RGBA, all layers)
    const pixelsPerLayer = this.atlasSize * this.atlasSize;
    this.atlasData = new Uint8Array(pixelsPerLayer * 4 * this.maxLayers);

    // Pre-allocate material data (4 floats per material: RGB + roughness)
    // 4096 materials max for efficient texture lookup
    this.materialDataArray = new Float32Array(4096 * 4);

    // Initialize with default material
    this.define(MATERIAL_DEFAULT, DEFAULT_MATERIAL);
  }

  /** Define or update a material */
  define(id: MaterialId, def: Partial<MaterialDef>): void {
    const existing = this.materials.get(id);
    const full: MaterialDef = {
      ...DEFAULT_MATERIAL,
      ...existing,
      ...def,
    };
    this.materials.set(id, full);
    this.updateMaterialData(id, full);
    this.notifyChange(id);
  }

  /** Get material definition */
  get(id: MaterialId): MaterialDef | undefined {
    return this.materials.get(id);
  }

  /** Update specific material properties */
  update(id: MaterialId, changes: Partial<MaterialDef>): void {
    const existing = this.materials.get(id);
    if (!existing) {
      throw new Error(`Material ${id} not defined`);
    }
    this.define(id, { ...existing, ...changes });
  }

  /** Delete a material */
  delete(id: MaterialId): void {
    if (id === MATERIAL_EMPTY || id === MATERIAL_DEFAULT) {
      throw new Error('Cannot delete reserved materials');
    }
    this.materials.delete(id);
    // Clear material data
    const offset = (id as number) * 4;
    this.materialDataArray.fill(0, offset, offset + 4);
    this.dataDirty = true;
    this.notifyChange(id);
  }

  /** Iterate all materials */
  entries(): IterableIterator<[MaterialId, MaterialDef]> {
    return this.materials.entries();
  }

  /** Get material count */
  count(): number {
    return this.materials.size;
  }

  /**
   * Load a texture image into the atlas for a material.
   * Image is scaled/cropped to tile size and placed at the material's atlas position.
   */
  async loadTexture(id: MaterialId, url: string): Promise<void> {
    const img = await this.loadImage(url);

    // Calculate atlas position
    const layer = Math.floor((id as number) / this.tilesPerLayer);
    const tileIndex = (id as number) % this.tilesPerLayer;
    const tileX = tileIndex % this.tilesPerAxis;
    const tileY = Math.floor(tileIndex / this.tilesPerAxis);

    if (layer >= this.maxLayers) {
      throw new Error(`Material ${id} exceeds atlas capacity (layer ${layer} >= ${this.maxLayers})`);
    }

    // Render image to canvas at tile size
    const canvas = document.createElement('canvas');
    canvas.width = this.tileSize;
    canvas.height = this.tileSize;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(img, 0, 0, this.tileSize, this.tileSize);
    const imageData = ctx.getImageData(0, 0, this.tileSize, this.tileSize);

    // Copy to atlas data
    this.copyTileToAtlas(imageData.data, layer, tileX, tileY);

    // Update material's texture reference
    const tileUvSize = 1 / this.tilesPerAxis;
    const def = this.materials.get(id) ?? { ...DEFAULT_MATERIAL };
    def.texture = {
      layer,
      uvMin: [tileX * tileUvSize, tileY * tileUvSize],
      uvMax: [(tileX + 1) * tileUvSize, (tileY + 1) * tileUvSize],
    };
    this.materials.set(id, def);

    this.atlasDirty = true;
    this.notifyChange(id);
  }

  /** Get the atlas texture (creates if needed, updates if dirty) */
  getAtlasTexture(): DataArrayTexture {
    if (!this.atlasTexture) {
      this.atlasTexture = new DataArrayTexture(
        this.atlasData,
        this.atlasSize,
        this.atlasSize,
        this.maxLayers
      );
      this.atlasTexture.format = RGBAFormat;
      this.atlasTexture.type = UnsignedByteType;
      this.atlasTexture.minFilter = NearestFilter;
      this.atlasTexture.magFilter = NearestFilter;
      this.atlasTexture.wrapS = RepeatWrapping;
      this.atlasTexture.wrapT = RepeatWrapping;
      this.atlasTexture.generateMipmaps = false;
      this.atlasTexture.needsUpdate = true;
      this.atlasDirty = false;
    } else if (this.atlasDirty) {
      this.atlasTexture.needsUpdate = true;
      this.atlasDirty = false;
    }
    return this.atlasTexture;
  }

  /** Get the material data texture (RGBA: baseR, baseG, baseB, roughness) */
  getDataTexture(): DataTexture {
    if (!this.dataTexture) {
      this.dataTexture = new DataTexture(
        this.materialDataArray,
        4096,
        1,
        RGBAFormat,
        FloatType
      );
      this.dataTexture.minFilter = NearestFilter;
      this.dataTexture.magFilter = NearestFilter;
      this.dataTexture.needsUpdate = true;
      this.dataDirty = false;
    } else if (this.dataDirty) {
      this.dataTexture.needsUpdate = true;
      this.dataDirty = false;
    }
    return this.dataTexture;
  }

  /** Register change callback, returns unsubscribe function */
  onChange(callback: (id: MaterialId) => void): () => void {
    this.changeCallbacks.add(callback);
    return () => this.changeCallbacks.delete(callback);
  }

  /** Dispose GPU resources */
  dispose(): void {
    this.atlasTexture?.dispose();
    this.dataTexture?.dispose();
    this.atlasTexture = null;
    this.dataTexture = null;
  }

  // --- Private methods ---

  private loadImage(url: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.crossOrigin = 'anonymous';
      img.onload = () => resolve(img);
      img.onerror = () => reject(new Error(`Failed to load texture: ${url}`));
      img.src = url;
    });
  }

  private copyTileToAtlas(
    pixels: Uint8ClampedArray,
    layer: number,
    tileX: number,
    tileY: number
  ): void {
    const pixelsPerLayer = this.atlasSize * this.atlasSize;
    const layerOffset = layer * pixelsPerLayer * 4;
    const startX = tileX * this.tileSize;
    const startY = tileY * this.tileSize;

    for (let y = 0; y < this.tileSize; y++) {
      for (let x = 0; x < this.tileSize; x++) {
        const srcIdx = (y * this.tileSize + x) * 4;
        const dstX = startX + x;
        const dstY = startY + y;
        const dstIdx = layerOffset + (dstY * this.atlasSize + dstX) * 4;

        this.atlasData[dstIdx] = pixels[srcIdx];
        this.atlasData[dstIdx + 1] = pixels[srcIdx + 1];
        this.atlasData[dstIdx + 2] = pixels[srcIdx + 2];
        this.atlasData[dstIdx + 3] = pixels[srcIdx + 3];
      }
    }
  }

  private updateMaterialData(id: MaterialId, def: MaterialDef): void {
    const offset = (id as number) * 4;
    this.materialDataArray[offset] = def.color[0];
    this.materialDataArray[offset + 1] = def.color[1];
    this.materialDataArray[offset + 2] = def.color[2];
    this.materialDataArray[offset + 3] = def.roughness;
    this.dataDirty = true;
  }

  private notifyChange(id: MaterialId): void {
    for (const callback of this.changeCallbacks) {
      callback(id);
    }
  }
}
```

### Atlas Building Utility

```typescript
/** Batch load multiple textures into the atlas */
export async function buildAtlas(
  registry: MaterialRegistry,
  textures: Array<{ id: MaterialId; url: string }>
): Promise<void> {
  // Load all textures in parallel
  const results = await Promise.allSettled(
    textures.map(async ({ id, url }) => {
      await registry.loadTexture(id, url);
      return id;
    })
  );

  // Report failures
  const failures = results.filter(r => r.status === 'rejected');
  if (failures.length > 0) {
    console.warn(`Failed to load ${failures.length} textures`);
  }
}

/** Define a color-only material (no texture) */
export function defineColorMaterial(
  registry: MaterialRegistry,
  id: MaterialId,
  color: [number, number, number],
  options?: { roughness?: number; metalness?: number }
): void {
  registry.define(id, {
    color: [color[0], color[1], color[2], 1.0],
    roughness: options?.roughness ?? 0.5,
    metalness: options?.metalness ?? 0.0,
    emissive: [0, 0, 0],
  });
}

/** Common material presets */
export const MaterialPresets = {
  stone: { color: [0.5, 0.5, 0.5, 1], roughness: 0.9, metalness: 0 },
  metal: { color: [0.8, 0.8, 0.9, 1], roughness: 0.2, metalness: 0.9 },
  wood: { color: [0.6, 0.4, 0.2, 1], roughness: 0.7, metalness: 0 },
  glass: { color: [0.9, 0.95, 1, 0.3], roughness: 0.1, metalness: 0 },
  grass: { color: [0.3, 0.6, 0.2, 1], roughness: 0.8, metalness: 0 },
  dirt: { color: [0.5, 0.35, 0.2, 1], roughness: 0.95, metalness: 0 },
} as const;
```

### Three.js Integration

```typescript
import { ShaderMaterial, DataArrayTexture, DataTexture, DoubleSide } from 'three';

/**
 * Create the voxel shader material for atlas-based rendering.
 * Handles:
 * - Texture atlas lookup with tiling
 * - Material property lookup (color, roughness)
 * - Basic diffuse + ambient lighting
 */
export function createVoxelMaterial(
  registry: MaterialRegistry
): ShaderMaterial {
  const atlas = registry.getAtlasTexture();
  const materialData = registry.getDataTexture();

  const material = new ShaderMaterial({
    uniforms: {
      atlas: { value: atlas },
      materialData: { value: materialData },
      tilesPerAxis: { value: 16 },
      tilesPerLayer: { value: 256 },
    },
    vertexShader: `
      attribute float materialId;

      varying vec2 vUv;
      varying float vMaterialId;
      varying vec3 vNormal;
      varying vec3 vViewPosition;

      void main() {
        vUv = uv;
        vMaterialId = materialId;
        vNormal = normalize(normalMatrix * normal);

        vec4 mvPosition = modelViewMatrix * vec4(position, 1.0);
        vViewPosition = -mvPosition.xyz;

        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: `
      precision highp float;
      precision highp sampler2DArray;

      uniform sampler2DArray atlas;
      uniform sampler2D materialData;
      uniform float tilesPerAxis;
      uniform float tilesPerLayer;

      varying vec2 vUv;
      varying float vMaterialId;
      varying vec3 vNormal;
      varying vec3 vViewPosition;

      void main() {
        // Material data lookup: RGBA = (R, G, B, roughness)
        float matLookup = (vMaterialId + 0.5) / 4096.0;
        vec4 matProps = texture2D(materialData, vec2(matLookup, 0.5));
        vec3 baseColor = matProps.rgb;
        float roughness = matProps.a;

        // Atlas position calculation
        float layer = floor(vMaterialId / tilesPerLayer);
        float tileIndex = mod(vMaterialId, tilesPerLayer);
        float tileX = mod(tileIndex, tilesPerAxis);
        float tileY = floor(tileIndex / tilesPerAxis);

        // UV to atlas coordinates with proper tiling
        vec2 tileSize = vec2(1.0 / tilesPerAxis);
        vec2 tileOffset = vec2(tileX, tileY) * tileSize;

        // fract(vUv) handles tiling for merged quads
        vec2 localUv = fract(vUv);
        vec2 atlasUv = localUv * tileSize + tileOffset;

        // Sample texture (or use white if no texture)
        vec4 texColor = texture(atlas, vec3(atlasUv, layer));

        // If texture is fully transparent, use base color
        vec3 albedo = texColor.a > 0.01 ? texColor.rgb * baseColor : baseColor;

        // Simple lighting model
        vec3 lightDir = normalize(vec3(0.5, 1.0, 0.3));
        vec3 viewDir = normalize(vViewPosition);
        vec3 halfDir = normalize(lightDir + viewDir);

        float NdotL = max(dot(vNormal, lightDir), 0.0);
        float NdotH = max(dot(vNormal, halfDir), 0.0);

        // Diffuse + specular based on roughness
        float specPower = mix(128.0, 4.0, roughness);
        float specular = pow(NdotH, specPower) * (1.0 - roughness) * 0.5;

        float ambient = 0.25;
        float diffuse = NdotL * 0.6;

        vec3 finalColor = albedo * (ambient + diffuse) + vec3(specular);

        gl_FragColor = vec4(finalColor, texColor.a > 0.01 ? texColor.a : 1.0);
      }
    `,
    side: DoubleSide,
    transparent: false,
  });

  // Update textures when materials change
  registry.onChange(() => {
    material.uniforms.atlas.value = registry.getAtlasTexture();
    material.uniforms.materialData.value = registry.getDataTexture();
  });

  return material;
}
```

### Usage Example

```typescript
// Initialize registry
const registry = new MaterialRegistry();

// Define materials
registry.define(materialId(1), MaterialPresets.stone);
registry.define(materialId(2), MaterialPresets.grass);
registry.define(materialId(3), MaterialPresets.dirt);

// Load textures for some materials
await buildAtlas(registry, [
  { id: materialId(1), url: '/textures/stone.png' },
  { id: materialId(2), url: '/textures/grass.png' },
]);

// Create shader material
const voxelMaterial = createVoxelMaterial(registry);

// Build mesh from WASM output
const meshData = wasmMesher.mesh_dense_voxels(voxelData, 64, 64, 64, 0.1, 0, 0, 0);
const geometry = buildVoxelGeometry({
  positions: new Float32Array(meshData.positions()),
  normals: new Float32Array(meshData.normals()),
  indices: new Uint32Array(meshData.indices()),
  uvs: new Float32Array(meshData.uvs()),
  materialIds: new Uint16Array(meshData.material_ids()),
});

const mesh = new Mesh(geometry, voxelMaterial);
scene.add(mesh);

// Runtime material changes are reflected immediately
registry.update(materialId(1), { color: [1, 0, 0, 1] });
```
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
