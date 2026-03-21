# Binary Greedy Meshing - Algorithm Analysis

> **Part of the Voxel Mesh Architecture**
>
> Deep technical analysis of the binary greedy meshing algorithm from [cgerikj/binary-greedy-meshing](https://github.com/cgerikj/binary-greedy-meshing).
>
> Related documents:
> - [Greedy Mesh Implementation](greedy-mesh-implementation-plan.md) - Our standard greedy meshing approach
> - [Architecture Overview](voxel-mesh-architecture.md) - High-level system design

---

## Overview

Binary greedy meshing is an optimization of traditional greedy meshing that uses bitwise operations to process multiple voxels in parallel. Instead of iterating voxel-by-voxel, it encodes voxel columns as 64-bit integers and performs visibility culling and merging using bit manipulation.

**Performance comparison:**

| Approach | Time per 32³ chunk | Operations per voxel |
|----------|-------------------|----------------------|
| Traditional greedy mesh | 1-5 ms | ~20-50 |
| Binary greedy mesh | 50-200 µs | ~0.3-1 (amortized via SIMD-like ops) |

The 10-50x speedup comes from processing 64 voxels per bitwise operation instead of one at a time.

---

## 1. Core Data Representation

### 1.1 The Opaque Mask

The algorithm represents voxel solidity using a **3D bitmask** where each bit indicates whether a voxel is solid:

```
Chunk dimensions: 64 × 64 × 64 (with 1-voxel padding = 62³ usable)
Storage: uint64_t opaqueMask[64 * 64]

Each uint64_t stores one vertical column (Y-axis):
  opaqueMask[x * 64 + z] = 64-bit column where bit N = voxel at (x, N, z)
```

**Memory footprint:**
- Traditional: 64³ bytes = 256 KB
- Bitmask: 64 × 64 × 8 bytes = 32 KB (8x reduction)

**Building the mask during decompression:**

```c
// From RLE-compressed data
void decompressToVoxelsAndOpaqueMask(
    const uint8_t* rleData,
    uint8_t* voxels,          // Full voxel type array
    uint64_t* opaqueMask      // Bit-packed solidity
) {
    int voxelIndex = 0;
    int rleIndex = 0;

    while (rleIndex < rleSize) {
        uint8_t type = rleData[rleIndex++];
        uint8_t length = rleData[rleIndex++];

        for (int i = 0; i < length; i++) {
            voxels[voxelIndex] = type;

            if (type != 0) {  // Non-air voxel
                int x = voxelIndex / (CS_P * CS_P);
                int y = (voxelIndex / CS_P) % CS_P;
                int z = voxelIndex % CS_P;

                // Set bit in column mask
                opaqueMask[x * CS_P + z] |= (1ULL << y);
            }
            voxelIndex++;
        }
    }
}
```

Where `CS_P = 64` (chunk size with padding) and `CS = 62` (usable chunk size).

### 1.2 Quad Encoding

Each merged quad is packed into a single 64-bit integer:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         64-bit Quad Layout                              │
├─────────────────────────────────────────────────────────────────────────┤
│ 63        32 │ 31  30 │ 29    24 │ 23    18 │ 17    12 │ 11   6 │ 5  0 │
│    type      │ unused │  height  │  width   │    z     │   y    │  x   │
│   (32 bits)  │(2 bits)│ (6 bits) │ (6 bits) │ (6 bits) │(6 bits)│(6b)  │
└─────────────────────────────────────────────────────────────────────────┘
```

**Encoding function:**

```c
uint64_t packQuad(
    uint64_t x, uint64_t y, uint64_t z,    // Position (0-63)
    uint64_t w, uint64_t h,                 // Dimensions (1-62)
    uint64_t type                           // Material ID
) {
    return (type << 32) | (h << 24) | (w << 18) | (z << 12) | (y << 6) | x;
}
```

**Why 8 bytes matters:**
- Traditional quad: 4 vertices × 3 floats × 4 bytes = 48 bytes
- Binary quad: 8 bytes (6x reduction)
- Cache-friendly: 8 quads fit in one 64-byte cache line

---

## 2. Face Culling via Bitwise Operations

### 2.1 The Culling Principle

A face is visible if:
1. The voxel is solid (bit set in its column)
2. The adjacent voxel is empty (bit clear in neighbor column)

In boolean terms: `visible = solid AND NOT neighbor_solid`

### 2.2 Column-Based Culling

The algorithm processes faces in column pairs. For the +Y face (top):

```c
// For each (x, z) column pair
for (int x = 1; x < CS_P - 1; x++) {
    for (int z = 1; z < CS_P - 1; z++) {
        int idx = x * CS_P + z;

        uint64_t thisColumn = opaqueMask[idx];
        uint64_t aboveColumn = opaqueMask[idx] >> 1;  // Shift = look at y+1

        // Face visible where: this is solid AND above is empty
        uint64_t topFaces = thisColumn & ~aboveColumn;

        // Remove padding bits
        topFaces = (topFaces >> 1) & ((1ULL << CS) - 1);

        faceMasks[idx] = topFaces;
    }
}
```

**Key insight:** The bit-shift `>> 1` effectively samples the neighbor voxel for all 64 Y positions in one operation.

### 2.3 All Six Face Directions

```c
void cullFaces(uint64_t* opaqueMask, uint64_t* faceMasks) {
    for (int a = 1; a < CS_P - 1; a++) {
        int aCS_P = a * CS_P;

        for (int b = 1; b < CS_P - 1; b++) {
            int abIndex = aCS_P + b;
            int baIndex = b * CS_P + a;

            uint64_t columnBits = opaqueMask[abIndex];

            // Face 0: +Y (top) - compare with column shifted up
            faceMasks[abIndex + 0 * CS_2] =
                (columnBits & ~(columnBits >> 1)) >> 1;

            // Face 1: -Y (bottom) - compare with column shifted down
            faceMasks[abIndex + 1 * CS_2] =
                (columnBits & ~(columnBits << 1)) >> 1;

            // Face 2: +X (right) - compare with x+1 column
            faceMasks[baIndex + 2 * CS_2] =
                (columnBits & ~opaqueMask[aCS_P + CS_P + b]) >> 1;

            // Face 3: -X (left) - compare with x-1 column
            faceMasks[baIndex + 3 * CS_2] =
                (columnBits & ~opaqueMask[aCS_P - CS_P + b]) >> 1;

            // Face 4: +Z (front) - compare with z+1 column
            faceMasks[abIndex + 4 * CS_2] =
                (columnBits & ~opaqueMask[aCS_P + b + 1]) >> 1;

            // Face 5: -Z (back) - compare with z-1 column
            faceMasks[abIndex + 5 * CS_2] =
                (columnBits & ~opaqueMask[aCS_P + b - 1]) >> 1;
        }
    }
}
```

Where `CS_2 = CS_P * CS_P = 4096` (stride between face direction arrays).

**Operations per chunk:**
- Traditional: 6 comparisons × 64³ voxels = 1,572,864 operations
- Binary: 6 comparisons × 64² columns = 24,576 operations (64x reduction)

---

## 3. Binary Greedy Merging

### 3.1 The Merging Problem

After culling, we have bitmasks indicating which faces are visible. Greedy merging combines adjacent same-material faces into larger rectangles.

**Traditional approach:** Nested loops scanning each position, O(n²) per layer.

**Binary approach:** Use bit-scanning intrinsics to jump directly to set bits.

### 3.2 Bit-Scanning Intrinsics

```c
// Count trailing zeros - finds position of lowest set bit
int firstSetBit = __builtin_ctzll(mask);  // GCC/Clang
// Or: _BitScanForward64(&firstSetBit, mask);  // MSVC

// Example:
// mask = 0b0001011000
// __builtin_ctzll(mask) = 3 (first '1' is at bit 3)
```

This allows jumping directly to visible faces instead of scanning every position.

### 3.3 Greedy Merge Algorithm

For each layer (slice perpendicular to face normal):

```c
void greedyMergeFace(
    int face,
    int layer,                    // Which slice we're processing
    uint64_t* faceMasks,          // Visibility bitmasks
    uint8_t* voxels,              // Material types
    std::vector<uint64_t>& quads  // Output
) {
    // Forward-merge tracking: how far each position merged in previous row
    int forwardMerged[CS] = {0};

    for (int row = 0; row < CS; row++) {
        int colIndex = getColumnIndex(face, layer, row);
        uint64_t rowBits = faceMasks[colIndex];

        int col = 0;
        while (rowBits != 0) {
            // Jump to next set bit
            int skip = __builtin_ctzll(rowBits);
            col += skip;
            rowBits >>= skip;

            if (col >= CS) break;

            // Get material type at this position
            uint8_t type = voxels[getVoxelIndex(face, layer, row, col)];

            // === Merge Right ===
            int width = 1;
            uint64_t scanBits = rowBits >> 1;

            while (scanBits & 1) {
                int nextCol = col + width;
                if (nextCol >= CS) break;

                // Check same material
                uint8_t nextType = voxels[getVoxelIndex(face, layer, row, nextCol)];
                if (nextType != type) break;

                // Check compatible forward merge
                if (forwardMerged[nextCol] != forwardMerged[col]) break;

                width++;
                scanBits >>= 1;
            }

            // === Merge Forward (from previous rows) ===
            int height = forwardMerged[col] + 1;

            // Update forward merge tracking
            for (int c = col; c < col + width; c++) {
                forwardMerged[c] = height;
            }

            // === Emit Quad (if this row completes a merge) ===
            bool canMergeMore = (row + 1 < CS) && /* check next row compatibility */;

            if (!canMergeMore) {
                // Emit the completed quad
                uint64_t quad = packQuad(
                    col, row - height + 1, layer,
                    width, height,
                    type
                );
                quads.push_back(quad);

                // Reset forward merge for these columns
                for (int c = col; c < col + width; c++) {
                    forwardMerged[c] = 0;
                }
            }

            // Clear processed bits
            rowBits &= ~((1ULL << width) - 1);
            col += width;
        }
    }
}
```

### 3.4 Merge Compatibility Tracking

The `forwardMerged[]` array tracks how many rows each column has successfully merged upward. This enables O(1) compatibility checking:

```
Row 0:  ████████  forwardMerged = [1,1,1,1,1,1,1,1]
Row 1:  ████████  forwardMerged = [2,2,2,2,2,2,2,2]  (all compatible)
Row 2:  ████░░██  forwardMerged = [3,3,3,3,0,0,1,1]  (gap breaks merge)
                                   ↑ emit 4×3 quad  ↑ start new merge
```

### 3.5 Different Merge Strategies by Face

The implementation uses two strategies:

**Strategy A (Faces 0-3: ±X, ±Y):**
- Row-major scanning with forward merge tracking
- Optimized for faces where columns align with memory layout

**Strategy B (Faces 4-5: ±Z):**
- Column-major scanning with right merge tracking
- Transposed access pattern for Z-aligned faces

```c
void meshFace(int face, /* ... */) {
    if (face < 4) {
        // Row-major merge (X/Y faces)
        for (int layer = 0; layer < CS; layer++) {
            mergeRowMajor(face, layer, faceMasks, voxels, quads);
        }
    } else {
        // Column-major merge (Z faces)
        for (int layer = 0; layer < CS; layer++) {
            mergeColumnMajor(face, layer, faceMasks, voxels, quads);
        }
    }
}
```

---

## 4. GPU Vertex Unpacking

### 4.1 Shader Storage Buffer Objects (SSBO)

Quads are uploaded to GPU memory as raw 64-bit integers:

```glsl
layout(std430, binding = 0) buffer QuadBuffer {
    uvec2 quads[];  // Each uvec2 = one 64-bit quad
};
```

### 4.2 Vertex Shader Unpacking

The vertex shader generates 4 vertices per quad using `gl_VertexID`:

```glsl
// Unpack position (6 bits each)
uint data = quads[gl_VertexID / 4].x;  // Lower 32 bits
ivec3 pos = ivec3(data, data >> 6, data >> 12) & 63;

// Unpack dimensions
int width = int((data >> 18) & 63);
int height = int((data >> 24) & 63);

// Unpack material type (upper 32 bits)
uint type = quads[gl_VertexID / 4].y;

// Generate corner based on vertex ID within quad
int corner = gl_VertexID & 3;  // 0, 1, 2, or 3
int wOffset = corner >> 1;      // 0, 0, 1, 1
int hOffset = corner & 1;       // 0, 1, 0, 1
```

### 4.3 Face-Dependent Coordinate Mapping

Each face direction maps width/height to different axes:

```glsl
// Face direction determines which axes width/height affect
const ivec3 wAxis[6] = ivec3[6](
    ivec3(1, 0, 0),  // +Y: width along X
    ivec3(1, 0, 0),  // -Y: width along X
    ivec3(0, 1, 0),  // +X: width along Y
    ivec3(0, 1, 0),  // -X: width along Y
    ivec3(1, 0, 0),  // +Z: width along X
    ivec3(1, 0, 0)   // -Z: width along X
);

const ivec3 hAxis[6] = ivec3[6](
    ivec3(0, 0, 1),  // +Y: height along Z
    ivec3(0, 0, 1),  // -Y: height along Z
    ivec3(0, 0, 1),  // +X: height along Z
    ivec3(0, 0, 1),  // -X: height along Z
    ivec3(0, 1, 0),  // +Z: height along Y
    ivec3(0, 1, 0)   // -Z: height along Y
);

// Apply width/height offsets
pos += wAxis[face] * width * wOffset;
pos += hAxis[face] * height * hOffset;
```

### 4.4 Winding Order and Face Flipping

Correct winding order for backface culling:

```glsl
// Flip width direction for negative faces to maintain CCW winding
const int flipLookup[6] = int[6](1, 1, 1, -1, 1, -1);

pos[wAxis[face]] += width * wOffset * flipLookup[face];
```

### 4.5 T-Junction Mitigation

To prevent hairline cracks between quads of different sizes:

```glsl
// Slightly expand quads in eye space
vec3 eyePos = pos - cameraPosition;
float expansion = 0.0007;

eyePos[wAxis[face]] += expansion * flipLookup[face] * (wOffset * 2 - 1);
eyePos[hAxis[face]] += expansion * (hOffset * 2 - 1);
```

This "fattens" each quad by a sub-pixel amount, ensuring edges overlap rather than gap.

---

## 5. Rendering Pipeline

### 5.1 Face-Grouped Draw Calls

Quads are sorted by face direction during meshing:

```c
struct ChunkMeshData {
    std::vector<uint64_t> vertices;

    // Per-face offsets into vertices array
    int faceBegin[6];
    int faceCount[6];
};
```

This enables:
1. **Face culling:** Skip drawing back-facing quads entirely
2. **Indirect drawing:** One draw call per visible face direction

### 5.2 Indirect Draw Commands

```c
struct DrawElementsIndirectCommand {
    uint count;          // Vertices to draw
    uint instanceCount;  // Always 1
    uint firstIndex;     // Starting index (0 for vertex pulling)
    uint baseVertex;     // Offset into SSBO
    uint baseInstance;   // Chunk ID for uniform access
};

// Generate commands for visible faces only
for (int face = 0; face < 6; face++) {
    if (isFaceVisible(face, cameraDir)) {
        commands.push_back({
            .count = meshData.faceCount[face] * 4,  // 4 verts per quad
            .instanceCount = 1,
            .firstIndex = 0,
            .baseVertex = meshData.faceBegin[face],
            .baseInstance = chunkId
        });
    }
}
```

### 5.3 Multi-threaded Chunk Processing

```
┌─────────────────────────────────────────────────────────────────┐
│                     Pipeline Stages                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Thread Pool (N workers)                                        │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐      │
│  │ Decomp  │───▶│  Mesh   │───▶│ Upload  │───▶│  Draw   │      │
│  │  RLE    │    │ Binary  │    │  GPU    │    │ Indirect│      │
│  │  ~10µs  │    │ ~70µs   │    │ ~20µs   │    │  <1ms   │      │
│  └─────────┘    └─────────┘    └─────────┘    └─────────┘      │
│       ↓              ↓              ↓                           │
│    [Task 1]      [Task 2]      [Task 3]      Main Thread       │
│    [Task 4]      [Task 5]      [Task 6]                        │
│       ...           ...           ...                           │
│                                                                  │
│  Throughput: ~5000 chunks/second on 8-core CPU                  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 6. Performance Analysis

### 6.1 Theoretical Complexity

| Operation | Traditional | Binary |
|-----------|-------------|--------|
| Face culling | O(n³) | O(n²) |
| Finding visible faces | O(n³) | O(visible faces) via CTZ |
| Greedy merge | O(n² × layers) | O(visible faces × log n) |
| Memory bandwidth | 256 KB | 32 KB (opaque mask) |

Where n = chunk dimension (typically 32-64).

### 6.2 Real-World Benchmarks

From the reference implementation (Ryzen 3800x, single-threaded):

| Chunk Content | Time | Quads Output |
|---------------|------|--------------|
| Empty chunk | ~5 µs | 0 |
| Solid cube | ~30 µs | 6 |
| Procedural terrain | ~74 µs | ~500-2000 |
| Complex cave system | ~150 µs | ~5000-10000 |

### 6.3 Memory Efficiency

For a 10×10×10 chunk render distance (1000 chunks):

| Data | Traditional | Binary |
|------|-------------|--------|
| Voxel storage | 256 MB | 32 MB (bitmasks) |
| Mesh output | ~50 MB | ~10 MB (8-byte quads) |
| Total GPU memory | ~300 MB | ~45 MB |

---

## 7. Adaptation Considerations

### 7.1 Material Support

The reference implementation stores material in the upper 32 bits of each quad. This limits materials to ~4 billion types, which is excessive. For our use case:

**Option A: 16-bit material ID**
```
Bits 0-17: Position (6+6+6)
Bits 18-29: Dimensions (6+6)
Bits 30-45: Material ID (16 bits = 65,536 materials)
Bits 46-63: Reserved (AO, flags, etc.)
```

**Option B: Separate material lookup**
```
Quad stores index into material palette
Material palette: separate uniform buffer
```

### 7.2 Ambient Occlusion

Per-vertex AO requires 4 values per quad (one per corner). Options:

1. **Expand to 128-bit quads:** Add second uint64 for AO + extra data
2. **Compute in shader:** Sample neighboring quads (slower but no memory cost)
3. **Texture lookup:** Bake AO into 3D texture

### 7.3 Chunk Boundary Handling

The reference uses 64³ chunks with 1-voxel padding on each side (62³ usable). For cross-chunk face visibility:

```
Chunk A (62³ usable)     Padding     Chunk B (62³ usable)
┌─────────────────────┐ ┌─────────┐ ┌─────────────────────┐
│                     │ │ A │ B   │ │                     │
│      Interior       │─│ p │ p   │─│      Interior       │
│       voxels        │ │ a │ a   │ │       voxels        │
│                     │ │ d │ d   │ │                     │
└─────────────────────┘ └─────────┘ └─────────────────────┘
                        ↑       ↑
                   Copy from neighbors
```

### 7.4 WASM Considerations

The algorithm relies on:

1. **64-bit integers:** Native in Rust/WASM
2. **CTZ intrinsic:** `u64::trailing_zeros()` in Rust
3. **Memory layout:** Ensure proper alignment for 64-bit access

```rust
// Rust equivalent of CTZ
let first_set_bit = mask.trailing_zeros() as usize;

// Pack quad in Rust
fn pack_quad(x: u32, y: u32, z: u32, w: u32, h: u32, mat: u32) -> u64 {
    ((mat as u64) << 32) | ((h as u64) << 24) | ((w as u64) << 18)
    | ((z as u64) << 12) | ((y as u64) << 6) | (x as u64)
}
```

---

## 8. Summary

Binary greedy meshing achieves 10-50x speedup over traditional greedy meshing through:

1. **Bitmask representation:** 8x memory reduction, cache-friendly access
2. **Bitwise face culling:** 64 voxels per operation instead of 1
3. **Bit-scanning intrinsics:** O(log n) to find next visible face
4. **8-byte quad packing:** Minimal GPU memory and bandwidth
5. **Shader-side unpacking:** No CPU vertex expansion

The tradeoffs:
- More complex implementation
- Fixed chunk size (must be ≤64 for 64-bit columns)
- Less flexibility for per-vertex attributes (need creative packing)

For our voxel mesh system, this approach is highly recommended for the core meshing algorithm, with our existing architecture handling chunk management, dirty tracking, and double-buffered swaps.

---

## References

- [cgerikj/binary-greedy-meshing](https://github.com/cgerikj/binary-greedy-meshing) - Reference implementation
- [0fps: Meshing in a Minecraft Game](https://0fps.net/2012/06/30/meshing-in-a-minecraft-game/) - Original greedy meshing article
- [Bit Twiddling Hacks](https://graphics.stanford.edu/~seander/bithacks.html) - CTZ and other bit operations
