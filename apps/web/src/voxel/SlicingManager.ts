/**
 * SlicingManager - Manages clipping planes for voxel visualization.
 *
 * Features:
 * - X/Y/Z axis slicing with configurable position
 * - Custom plane support for arbitrary slicing
 * - Enable/disable toggle without affecting plane positions
 * - Works with both WebGL and WebGPU renderers
 *
 * Note: Slicing uses GPU clipping planes, so it doesn't require
 * geometry rebuilds - updates are immediate.
 */

import { Plane, Vector3, Material } from "three";
import type { SliceAxis, SliceConfig, SlicingConfig } from "./types";

/**
 * Default slice configuration.
 */
const defaultSliceConfig = (): SliceConfig => ({
  axis: "y",
  position: 0,
  direction: 1,
  enabled: false,
});

/**
 * SlicingManager manages GPU clipping planes for voxel visualization.
 *
 * Clipping planes are applied at the material level, so changes are
 * immediate without requiring geometry rebuilds.
 */
export class SlicingManager {
  /** Clipping planes for each axis */
  private planes: Map<SliceAxis, Plane> = new Map();

  /** Current slice configurations */
  private configs: Map<SliceAxis, SliceConfig> = new Map();

  /** Materials to apply clipping to */
  private materials: Set<Material> = new Set();

  /** Global enable/disable */
  private enabled = false;

  constructor() {
    // Initialize planes for each axis
    this.initPlane("x", new Vector3(1, 0, 0));
    this.initPlane("y", new Vector3(0, 1, 0));
    this.initPlane("z", new Vector3(0, 0, 1));
  }

  /**
   * Initialize a clipping plane for an axis.
   */
  private initPlane(axis: SliceAxis, normal: Vector3): void {
    const plane = new Plane(normal, 0);
    this.planes.set(axis, plane);
    this.configs.set(axis, defaultSliceConfig());
  }

  // ========================================================================
  // Configuration
  // ========================================================================

  /**
   * Set the slicing configuration.
   */
  setConfig(config: Partial<SlicingConfig>): void {
    if (config.enabled !== undefined) {
      this.enabled = config.enabled;
    }

    if (config.slices) {
      for (const axis of ["x", "y", "z"] as SliceAxis[]) {
        const sliceConfig = config.slices[axis];
        if (sliceConfig) {
          this.setSlice(axis, sliceConfig);
        }
      }
    }

    this.updateMaterials();
  }

  /**
   * Set configuration for a single axis slice.
   */
  setSlice(axis: SliceAxis, config: Partial<SliceConfig>): void {
    const existing = this.configs.get(axis) ?? defaultSliceConfig();
    const updated: SliceConfig = {
      ...existing,
      ...config,
      axis,
    };
    this.configs.set(axis, updated);
    this.updatePlane(axis);
    this.updateMaterials();
  }

  /**
   * Update a plane's position and direction.
   */
  private updatePlane(axis: SliceAxis): void {
    const plane = this.planes.get(axis);
    const config = this.configs.get(axis);

    if (!plane || !config) return;

    // Set normal direction based on axis and clip direction
    const normals: Record<SliceAxis, Vector3> = {
      x: new Vector3(1, 0, 0),
      y: new Vector3(0, 1, 0),
      z: new Vector3(0, 0, 1),
    };

    plane.normal.copy(normals[axis]);
    if (config.direction < 0) {
      plane.normal.negate();
    }

    // Set plane constant (distance from origin)
    // Plane equation: normal.dot(point) + constant = 0
    // For clipping at position P: constant = -normal.dot(P)
    plane.constant = -config.position * config.direction;
  }

  /**
   * Enable or disable all slicing.
   */
  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    this.updateMaterials();
  }

  /**
   * Check if slicing is enabled.
   */
  isEnabled(): boolean {
    return this.enabled;
  }

  // ========================================================================
  // Slice Position Control
  // ========================================================================

  /**
   * Set the slice position for an axis.
   */
  setSlicePosition(axis: SliceAxis, position: number): void {
    const config = this.configs.get(axis);
    if (config) {
      config.position = position;
      this.updatePlane(axis);
    }
  }

  /**
   * Get the slice position for an axis.
   */
  getSlicePosition(axis: SliceAxis): number {
    return this.configs.get(axis)?.position ?? 0;
  }

  /**
   * Set the slice direction for an axis.
   */
  setSliceDirection(axis: SliceAxis, direction: 1 | -1): void {
    const config = this.configs.get(axis);
    if (config) {
      config.direction = direction;
      this.updatePlane(axis);
    }
  }

  /**
   * Enable or disable a single axis slice.
   */
  setSliceEnabled(axis: SliceAxis, enabled: boolean): void {
    const config = this.configs.get(axis);
    if (config) {
      config.enabled = enabled;
      this.updateMaterials();
    }
  }

  /**
   * Check if a slice axis is enabled.
   */
  isSliceEnabled(axis: SliceAxis): boolean {
    return this.configs.get(axis)?.enabled ?? false;
  }

  // ========================================================================
  // Material Management
  // ========================================================================

  /**
   * Register a material to receive clipping plane updates.
   */
  addMaterial(material: Material): void {
    this.materials.add(material);
    this.applyClippingToMaterial(material);
  }

  /**
   * Remove a material from clipping updates.
   */
  removeMaterial(material: Material): void {
    this.materials.delete(material);
    // Clear clipping planes from material
    material.clippingPlanes = null;
  }

  /**
   * Update all registered materials with current clipping planes.
   */
  private updateMaterials(): void {
    for (const material of this.materials) {
      this.applyClippingToMaterial(material);
    }
  }

  /**
   * Apply current clipping configuration to a material.
   */
  private applyClippingToMaterial(material: Material): void {
    if (!this.enabled) {
      material.clippingPlanes = null;
      return;
    }

    const activePlanes: Plane[] = [];

    for (const axis of ["x", "y", "z"] as SliceAxis[]) {
      const config = this.configs.get(axis);
      const plane = this.planes.get(axis);

      if (config?.enabled && plane) {
        activePlanes.push(plane);
      }
    }

    if (activePlanes.length > 0) {
      material.clippingPlanes = activePlanes;
      material.clipIntersection = false; // Clip if outside ANY plane
    } else {
      material.clippingPlanes = null;
    }
  }

  // ========================================================================
  // State Inspection
  // ========================================================================

  /**
   * Get the current configuration.
   */
  getConfig(): SlicingConfig {
    return {
      enabled: this.enabled,
      slices: {
        x: this.configs.get("x"),
        y: this.configs.get("y"),
        z: this.configs.get("z"),
      },
    };
  }

  /**
   * Get active clipping planes.
   */
  getActivePlanes(): Plane[] {
    if (!this.enabled) return [];

    const planes: Plane[] = [];
    for (const axis of ["x", "y", "z"] as SliceAxis[]) {
      const config = this.configs.get(axis);
      const plane = this.planes.get(axis);
      if (config?.enabled && plane) {
        planes.push(plane);
      }
    }
    return planes;
  }

  // ========================================================================
  // Disposal
  // ========================================================================

  /**
   * Clean up resources.
   */
  dispose(): void {
    // Clear clipping planes from all materials
    for (const material of this.materials) {
      material.clippingPlanes = null;
    }
    this.materials.clear();
    this.planes.clear();
    this.configs.clear();
  }
}
