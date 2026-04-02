# Material Table

**Type:** spec
**Status:** current
**Date:** 2026-03-22
**Category:** Authoritative data — never derived, written by producers only.

> Global material palette. Maps a MaterialId (u16) to packed PBR properties. CPU-authored, GPU-read.

---

## Identity

- **Buffer name:** `material_table`
- **WGSL type:** `array<MaterialEntry, 4096>` where `MaterialEntry` is 4 × `u32` (packed f16 pairs)
- **GPU usage:** `STORAGE` (read-only from GPU; CPU writes via `writeBuffer`)
- **Binding:** `@group(0) @binding(?)` in fragment, summary, and cascade shaders

---

## Layout

The table holds **4,096 entries** of **16 bytes** each, for a total of **65,536 bytes** (64 KB).

### MaterialEntry struct (16 bytes)

```
struct MaterialEntry {
    albedo_rg:          u32,   // bits 0-15 = albedo.r (f16), bits 16-31 = albedo.g (f16)
    albedo_b_roughness: u32,   // bits 0-15 = albedo.b (f16), bits 16-31 = roughness (f16)
    emissive_rg:        u32,   // bits 0-15 = emissive.r (f16), bits 16-31 = emissive.g (f16)
    emissive_b_opacity: u32,   // bits 0-15 = emissive.b (f16), bits 16-31 = opacity (f16)
};
```

### Field extraction (WGSL)

```
fn unpack_f16_lo(packed: u32) -> f32 { return unpack2x16float(packed).x; }
fn unpack_f16_hi(packed: u32) -> f32 { return unpack2x16float(packed).y; }

fn get_albedo(e: MaterialEntry) -> vec3<f32> {
    return vec3(
        unpack_f16_lo(e.albedo_rg),
        unpack_f16_hi(e.albedo_rg),
        unpack_f16_lo(e.albedo_b_roughness)
    );
}

fn get_roughness(e: MaterialEntry) -> f32 { return unpack_f16_hi(e.albedo_b_roughness); }
fn get_opacity(e: MaterialEntry)   -> f32 { return unpack_f16_hi(e.emissive_b_opacity); }
```

### Reserved MaterialIds

| ID | Name | Description |
|---|---|---|
| 0 | `MATERIAL_EMPTY` | Void / air. All fields zero. Never rendered. |
| 1 | `MATERIAL_DEFAULT` | Fallback material. Albedo (0.5, 0.5, 0.5), roughness 0.5, opacity 1.0, emissive zero. |
| 2-4095 | Producer-assigned | Available for scene manager allocation. |

### Fast emissive test

```
fn mat_is_emissive(e: MaterialEntry) -> bool {
    return (e.emissive_rg != 0u) || ((e.emissive_b_opacity & 0xFFFFu) != 0u);
}
```

---

## Invariants

| ID | Invariant | Enforced by |
|---|---|---|
| MAT-1 | Albedo channels in [0, 1] | CPU validation on registration |
| MAT-2 | Roughness in [0, 1] | CPU validation on registration |
| MAT-3 | Opacity in [0, 1] | CPU validation on registration |
| MAT-4 | Emissive channels in [0, +inf) (f16 range) | CPU validation; f16 max ~ 65504 |
| MAT-5 | `material_table[0]` is `MATERIAL_EMPTY` — all fields zero | Scene manager initialization |
| MAT-6 | `material_table[1]` is `MATERIAL_DEFAULT` — albedo (0.5, 0.5, 0.5), roughness 0.5, opacity 1.0 | Scene manager initialization |
| MAT-7 | Total buffer size is exactly 65,536 bytes | Buffer creation (4096 × 16) |

---

## Valid Value Ranges

| Field | Range | Notes |
|---|---|---|
| `MaterialId` | `0 .. 4095` | 12-bit index; matches palette entry width |
| `albedo.r`, `albedo.g`, `albedo.b` | `0.0 .. 1.0` (f16) | Linear color space |
| `roughness` | `0.0 .. 1.0` (f16) | 0 = mirror, 1 = fully rough |
| `emissive.r`, `emissive.g`, `emissive.b` | `0.0 .. 65504.0` (f16) | HDR emissive; f16 max is 65504 |
| `opacity` | `0.0 .. 1.0` (f16) | 0 = fully transparent, 1 = fully opaque |

---

## Invalidation on Property Change

When a material's properties are updated at runtime:

1. CPU writes new `MaterialEntry` to the corresponding slot via `writeBuffer`.
2. CPU increments `material_table_version`.
3. CPU sets `stale_summary` for all resident slots (since any slot's palette may reference the changed material).
4. I-3 re-scans palettes on next frame to propagate changes to `chunk_flags` and emissive metadata.

---

## Producers

| Producer | When | What it writes |
|---|---|---|
| Scene manager | Material registration / property change | Full `MaterialEntry` at the assigned ID via CPU `writeBuffer` |

---

## Consumers

| Consumer | Stage | How it reads |
|---|---|---|
| Summary rebuild | I-3 | Reads palette entries to scan for emissive materials |
| Fragment shader | R-5 | Fetches `MaterialEntry` by `MaterialId` for PBR shading |
| Cascade build | R-6 | Reads emissive channels for emissive hit test during ray march |

---

## Testing Strategy

### Unit tests (Rust, CPU-side)

1. **Reserved ID contents:** Verify `material_table[0]` is all-zero and `material_table[1]` matches `MATERIAL_DEFAULT` values.
2. **Pack/unpack roundtrip:** For random albedo, roughness, emissive, opacity values, pack to 4 × u32, unpack, verify values match within f16 precision.
3. **Range clamping:** Attempt to register albedo > 1.0 or opacity < 0.0 — verify validation rejects or clamps.
4. **Fast emissive test:** Verify `mat_is_emissive` returns false for zero emissive and true for any non-zero emissive channel.

### Property tests (Rust, randomized)

5. **Exhaustive ID range:** Write random materials to all 4096 slots, read back, every entry matches.
6. **Entry isolation:** Writing to ID N does not affect ID N+1 or N-1.

### GPU validation (WGSL compute)

7. **Readback test:** CPU writes known materials, dispatch compute shader that reads each entry and writes unpacked values to a readback buffer — verify against CPU reference.
8. **Emissive scan test:** Populate table with a mix of emissive and non-emissive entries, dispatch shader that runs `mat_is_emissive` on all 4096 — verify bitmask matches CPU expectation.
