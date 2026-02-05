/**
 * Tiered BufferGeometry pool for reusing GPU buffers.
 *
 * Pre-allocates buffers at tier capacity and uses `setDrawRange()` to
 * avoid reallocation when actual vertex/index counts are smaller than
 * the tier maximum. This reduces WebGL buffer churn and GC pressure
 * when chunks are frequently rebuilt.
 */

import {
  BufferAttribute,
  BufferGeometry,
  Box3,
  Sphere,
  Vector3,
} from "three";

// ---------------------------------------------------------------------------
// Tier configuration
// ---------------------------------------------------------------------------

/** Tier sizes based on vertex capacity. Index capacity assumes ~1.5 indices/vertex (quad topology). */
const TIER_CONFIGS = [
  { name: "tiny" as const, maxVertices: 256, maxIndices: 384 },
  { name: "small" as const, maxVertices: 1024, maxIndices: 1536 },
  { name: "medium" as const, maxVertices: 4096, maxIndices: 6144 },
  { name: "large" as const, maxVertices: 16384, maxIndices: 24576 },
  { name: "huge" as const, maxVertices: 65536, maxIndices: 98304 },
] as const;

type TierName = (typeof TIER_CONFIGS)[number]["name"];

type PooledEntry = {
  geometry: BufferGeometry;
  tier: TierName;
  inUse: boolean;
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

export type GeometryPoolStats = {
  totalAllocated: number;
  inUse: number;
  available: number;
  byTier: Record<TierName, { allocated: number; inUse: number }>;
};

// ---------------------------------------------------------------------------
// GeometryPool
// ---------------------------------------------------------------------------

export class GeometryPool {
  private tiers: Map<TierName, PooledEntry[]> = new Map();
  private lookup: Map<BufferGeometry, PooledEntry> = new Map();
  private maxPerTier: number;
  private enabled: boolean;

  constructor(options?: { maxPerTier?: number; enabled?: boolean }) {
    this.maxPerTier = options?.maxPerTier ?? 16;
    this.enabled = options?.enabled ?? true;

    for (const tier of TIER_CONFIGS) {
      this.tiers.set(tier.name, []);
    }
  }

  /**
   * Acquire a geometry with pre-allocated buffers for the given counts.
   *
   * If the pool has an available geometry in the matching tier it is
   * returned directly. Otherwise a new geometry is created with
   * buffers sized to the tier capacity.
   *
   * If vertex/index counts exceed all tiers, returns an unpooled
   * geometry that will be disposed (not recycled) on release.
   */
  acquire(vertexCount: number, indexCount: number): BufferGeometry {
    if (!this.enabled) {
      return new BufferGeometry();
    }

    const tier = this.findTier(vertexCount, indexCount);
    if (!tier) {
      return new BufferGeometry();
    }

    const pool = this.tiers.get(tier.name)!;

    for (const entry of pool) {
      if (!entry.inUse) {
        entry.inUse = true;
        return entry.geometry;
      }
    }

    const geometry = this.createTierGeometry(tier);
    const entry: PooledEntry = { geometry, tier: tier.name, inUse: true };
    pool.push(entry);
    this.lookup.set(geometry, entry);

    return geometry;
  }

  /**
   * Release a geometry back to the pool.
   *
   * If the geometry is not tracked by this pool (e.g. oversized) or
   * the pool is disabled, it is disposed immediately. Excess entries
   * beyond `maxPerTier` are also disposed.
   */
  release(geometry: BufferGeometry): void {
    const entry = this.lookup.get(geometry);
    if (!entry || !this.enabled) {
      geometry.dispose();
      return;
    }

    entry.inUse = false;

    const pool = this.tiers.get(entry.tier)!;
    const availableCount = pool.filter((e) => !e.inUse).length;

    if (availableCount > this.maxPerTier) {
      const idx = pool.indexOf(entry);
      if (idx >= 0) {
        pool.splice(idx, 1);
        this.lookup.delete(geometry);
        geometry.dispose();
      }
    }
  }

  /** Dispose all pooled geometries and reset. */
  dispose(): void {
    for (const pool of this.tiers.values()) {
      for (const entry of pool) {
        entry.geometry.dispose();
      }
      pool.length = 0;
    }
    this.lookup.clear();
  }

  /** Get pool statistics. */
  getStats(): GeometryPoolStats {
    let totalAllocated = 0;
    let inUse = 0;
    const byTier = {} as Record<
      TierName,
      { allocated: number; inUse: number }
    >;

    for (const tier of TIER_CONFIGS) {
      const pool = this.tiers.get(tier.name)!;
      const tierInUse = pool.filter((e) => e.inUse).length;
      byTier[tier.name] = { allocated: pool.length, inUse: tierInUse };
      totalAllocated += pool.length;
      inUse += tierInUse;
    }

    return { totalAllocated, inUse, available: totalAllocated - inUse, byTier };
  }

  // -------------------------------------------------------------------------
  // Private
  // -------------------------------------------------------------------------

  private findTier(vertexCount: number, indexCount: number) {
    for (const tier of TIER_CONFIGS) {
      if (vertexCount <= tier.maxVertices && indexCount <= tier.maxIndices) {
        return tier;
      }
    }
    return null;
  }

  private createTierGeometry(
    tier: (typeof TIER_CONFIGS)[number],
  ): BufferGeometry {
    const geometry = new BufferGeometry();

    geometry.setAttribute(
      "position",
      new BufferAttribute(new Float32Array(tier.maxVertices * 3), 3),
    );
    geometry.setAttribute(
      "normal",
      new BufferAttribute(new Float32Array(tier.maxVertices * 3), 3),
    );
    geometry.setAttribute(
      "uv",
      new BufferAttribute(new Float32Array(tier.maxVertices * 2), 2),
    );
    geometry.setAttribute(
      "materialId",
      new BufferAttribute(new Uint16Array(tier.maxVertices), 1),
    );
    geometry.setIndex(
      new BufferAttribute(new Uint32Array(tier.maxIndices), 1),
    );

    geometry.setDrawRange(0, 0);
    return geometry;
  }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/** Reusable Vector3 for bounding-volume computation. */
const _v = new Vector3();

/**
 * Fill a (possibly pooled) geometry with mesh data.
 *
 * Copies data into existing pre-allocated buffers when possible,
 * falling back to new BufferAttributes for unpooled geometries.
 * Sets the draw range and computes bounding volumes from the
 * actual vertex data only (ignoring unused buffer capacity).
 */
export function fillGeometry(
  geometry: BufferGeometry,
  data: {
    positions: Float32Array;
    normals: Float32Array;
    indices: Uint32Array;
    uvs?: Float32Array;
    materialIds?: Uint16Array;
    vertexCount: number;
  },
): void {
  // Positions
  const posAttr = geometry.getAttribute("position") as BufferAttribute | null;
  if (posAttr && posAttr.array.length >= data.positions.length) {
    (posAttr.array as Float32Array).set(data.positions);
    posAttr.needsUpdate = true;
  } else {
    geometry.setAttribute(
      "position",
      new BufferAttribute(data.positions, 3),
    );
  }

  // Normals
  const normAttr = geometry.getAttribute("normal") as BufferAttribute | null;
  if (normAttr && normAttr.array.length >= data.normals.length) {
    (normAttr.array as Float32Array).set(data.normals);
    normAttr.needsUpdate = true;
  } else {
    geometry.setAttribute("normal", new BufferAttribute(data.normals, 3));
  }

  // Indices
  const indexAttr = geometry.getIndex();
  if (indexAttr && indexAttr.array.length >= data.indices.length) {
    (indexAttr.array as Uint32Array).set(data.indices);
    indexAttr.needsUpdate = true;
  } else {
    geometry.setIndex(new BufferAttribute(data.indices, 1));
  }

  // UVs (optional)
  if (data.uvs && data.uvs.length > 0) {
    const uvAttr = geometry.getAttribute("uv") as BufferAttribute | null;
    if (uvAttr && uvAttr.array.length >= data.uvs.length) {
      (uvAttr.array as Float32Array).set(data.uvs);
      uvAttr.needsUpdate = true;
    } else {
      geometry.setAttribute("uv", new BufferAttribute(data.uvs, 2));
    }
  }

  // Material IDs (optional)
  if (data.materialIds && data.materialIds.length > 0) {
    const matAttr = geometry.getAttribute(
      "materialId",
    ) as BufferAttribute | null;
    if (matAttr && matAttr.array.length >= data.materialIds.length) {
      (matAttr.array as Uint16Array).set(data.materialIds);
      matAttr.needsUpdate = true;
    } else {
      geometry.setAttribute(
        "materialId",
        new BufferAttribute(data.materialIds, 1),
      );
    }
  }

  // Draw range: only render the actual data, not the full tier capacity
  geometry.setDrawRange(0, data.indices.length);

  // Compute bounding volumes from actual vertex data only
  const box = new Box3();
  for (let i = 0; i < data.vertexCount * 3; i += 3) {
    _v.set(data.positions[i], data.positions[i + 1], data.positions[i + 2]);
    box.expandByPoint(_v);
  }
  geometry.boundingBox = box;
  const sphere = new Sphere();
  box.getBoundingSphere(sphere);
  geometry.boundingSphere = sphere;
}

/**
 * Estimate GPU memory usage for a geometry in bytes.
 *
 * Sums the byte lengths of all attribute arrays and the index array.
 */
export function estimateGeometryMemory(geometry: BufferGeometry): number {
  let bytes = 0;
  for (const name of Object.keys(geometry.attributes)) {
    const attr = geometry.getAttribute(name);
    if (attr && "array" in attr) {
      bytes += (attr as BufferAttribute).array.byteLength;
    }
  }
  const index = geometry.getIndex();
  if (index) {
    bytes += index.array.byteLength;
  }
  return bytes;
}
