import {
  AxesHelper,
  Box3,
  Box3Helper,
  GridHelper,
  Group,
  Mesh,
  MeshBasicMaterial,
  Vector3
} from "three";
import type { ModuleOutput } from "../modules/types";
import type { ViewerBackend } from "./threeBackend";
import { buildOutputObject, computeStats } from "./outputs";

const LOG_VIEWER = false;

export type ViewerOptions = {
  overlay: HTMLElement;
  testMode: boolean;
};

export class Viewer {
  private outputGroup = new Group();
  private grid = new GridHelper(10, 10, 0x2a2f39, 0x1b2028);
  private axes = new AxesHelper(2.5);
  private bounds = new Box3();
  private boundsHelper = new Box3Helper(this.bounds, 0x7ad8ff);
  private boundsVisible = false;
  private wireframe = false;
  private unlit = false;
  private stats = { triangles: 0, instances: 0 };
  private autoFrameOnNextOutput = true;
  private chunkObjects = new Map<string, import("three").Object3D>();

  constructor(private backend: ViewerBackend, private options: ViewerOptions) {
    this.grid.visible = false;
    this.axes.visible = true;
    this.backend.scene.add(this.grid);
    this.backend.scene.add(this.axes);
    this.boundsHelper.visible = false;
    this.backend.scene.add(this.boundsHelper);
    this.backend.scene.add(this.outputGroup);
  }

  resize(width: number, height: number): void {
    this.backend.resize(width, height);
  }

  render(): void {
    this.backend.render();
  }

  setOutputs(outputs: ModuleOutput[]): void {
    this.disposeOutputGroup();
    this.outputGroup.clear();
    const renderStart = performance.now();
    let lastOverlayUpdate = 0;
    const onChunkAdded = () => {
      const now = performance.now();
      if (now - lastOverlayUpdate < 250) {
        return;
      }
      lastOverlayUpdate = now;
      this.stats = computeStats(this.outputGroup);
      this.updateOverlay();
    };
    for (const output of outputs) {
      const { object } = buildOutputObject(output);
      this.outputGroup.add(object);
      if (
        output.kind === "voxels" &&
        output.voxels.renderMode === "cubes" &&
        object instanceof Group
      ) {
        object.userData.onChunkAdded = onChunkAdded;
      }
    }
    this.applyWireframe();
    this.applyMaterialMode();
    this.bounds.setFromObject(this.outputGroup);
    this.boundsHelper.box.copy(this.bounds);
    this.boundsHelper.visible = this.boundsVisible && !this.bounds.isEmpty();
    if (this.autoFrameOnNextOutput && !this.bounds.isEmpty()) {
      this.frameObject();
      this.autoFrameOnNextOutput = false;
    }
    this.stats = computeStats(this.outputGroup);
    this.updateOverlay();
    const renderMs = performance.now() - renderStart;
    if (LOG_VIEWER && renderMs > 32) {
      console.log("[viewer] outputs built", `ms=${renderMs.toFixed(1)}`);
    }
  }

  /**
   * Append outputs to the scene keyed by label.
   * If a key already exists, the old object is disposed and replaced.
   */
  appendOutputs(outputs: ModuleOutput[]): void {
    const wasEmpty = this.chunkObjects.size === 0;

    for (const output of outputs) {
      const key = output.label ?? `_anon_${this.chunkObjects.size}`;

      // Replace existing object with same key
      const existing = this.chunkObjects.get(key);
      if (existing) {
        this.disposeObject(existing);
        this.outputGroup.remove(existing);
      }

      const { object } = buildOutputObject(output);
      object.name = key;
      this.outputGroup.add(object);
      this.chunkObjects.set(key, object);
    }

    this.applyWireframe();
    this.applyMaterialMode();
    this.bounds.setFromObject(this.outputGroup);
    this.boundsHelper.box.copy(this.bounds);
    this.boundsHelper.visible = this.boundsVisible && !this.bounds.isEmpty();

    // Auto-frame only on the first batch
    if (wasEmpty && this.autoFrameOnNextOutput && !this.bounds.isEmpty()) {
      this.frameObject();
      this.autoFrameOnNextOutput = false;
    }

    this.stats = computeStats(this.outputGroup);
    this.updateOverlay();
  }

  /** Clear all keyed chunk objects from the scene. */
  clearChunkOutputs(): void {
    for (const obj of this.chunkObjects.values()) {
      this.disposeObject(obj);
      this.outputGroup.remove(obj);
    }
    this.chunkObjects.clear();
    this.stats = computeStats(this.outputGroup);
    this.updateOverlay();
  }

  setWireframe(enabled: boolean): void {
    this.wireframe = enabled;
    this.applyWireframe();
  }

  setUnlit(enabled: boolean): void {
    this.unlit = enabled;
    this.applyMaterialMode();
  }

  setGridVisible(visible: boolean): void {
    this.grid.visible = visible;
  }

  setAxesVisible(visible: boolean): void {
    this.axes.visible = visible;
  }

  setBoundsVisible(visible: boolean): void {
    this.boundsVisible = visible;
    this.boundsHelper.visible = visible && !this.bounds.isEmpty();
  }

  frameObject(): void {
    if (this.bounds.isEmpty()) {
      return;
    }
    const size = this.bounds.getSize(new Vector3()).length();
    const center = this.bounds.getCenter(new Vector3());
    const distance = size * 0.001;
    this.backend.camera.position.copy(center.clone().add(new Vector3(0.01, -0.10, .01)));
    this.backend.camera.lookAt(new Vector3(0, -0.1, 0));
    this.backend.controls.target.copy(center);
    this.backend.controls.update();
  }

  updateOverlay(): void {
    this.options.overlay.textContent = `Triangles: ${Math.round(
      this.stats.triangles
    )} | Instances: ${Math.round(this.stats.instances)}`;
  }

  getStats() {
    return this.stats;
  }

  private applyWireframe(): void {
    this.outputGroup.traverse((child) => {
      const material = (child as { material?: { wireframe?: boolean } }).material;
      if (material && "wireframe" in material) {
        material.wireframe = this.wireframe;
      }
    });
  }

  private applyMaterialMode(): void {
    this.outputGroup.traverse((child) => {
      if (!(child instanceof Mesh)) {
        return;
      }

      const mesh = child as Mesh;
      const material = mesh.material as MeshBasicMaterial | MeshBasicMaterial[];
      if (this.unlit) {
        if ((mesh.userData as { litMaterial?: unknown }).litMaterial) {
          return;
        }
        const source = Array.isArray(material) ? material[0] : material;
        const basic = new MeshBasicMaterial({
          color: source.color?.clone?.() ?? 0xffffff,
          vertexColors: Boolean(source.vertexColors)
        });
        mesh.userData.litMaterial = mesh.material;
        mesh.material = basic;
      } else if ((mesh.userData as { litMaterial?: unknown }).litMaterial) {
        mesh.material = (mesh.userData as { litMaterial?: unknown }).litMaterial as MeshBasicMaterial;
        delete (mesh.userData as { litMaterial?: unknown }).litMaterial;
      }
    });
  }

  private disposeObject(obj: import("three").Object3D): void {
    obj.traverse((child) => {
      const geometry = (child as { geometry?: { dispose?: () => void } }).geometry;
      geometry?.dispose?.();
      const material = (child as { material?: unknown }).material;
      if (Array.isArray(material)) {
        for (const entry of material) {
          this.disposeMaterial(entry as { map?: { dispose?: () => void }; dispose?: () => void });
        }
      } else if (material) {
        this.disposeMaterial(material as { map?: { dispose?: () => void }; dispose?: () => void });
      }
    });
  }

  private disposeOutputGroup(): void {
    this.outputGroup.traverse((child) => {
      if (child.userData && "buildToken" in child.userData) {
        child.userData.buildToken = null;
      }
      const geometry = (child as { geometry?: { dispose?: () => void } }).geometry;
      geometry?.dispose?.();
      const material = (child as { material?: unknown }).material;
      if (Array.isArray(material)) {
        for (const entry of material) {
          this.disposeMaterial(entry as { map?: { dispose?: () => void }; dispose?: () => void });
        }
      } else if (material) {
        this.disposeMaterial(material as { map?: { dispose?: () => void }; dispose?: () => void });
      }
      const litMaterial = (child as { userData?: { litMaterial?: unknown } }).userData?.litMaterial;
      if (litMaterial) {
        if (Array.isArray(litMaterial)) {
          for (const entry of litMaterial) {
            this.disposeMaterial(entry as { map?: { dispose?: () => void }; dispose?: () => void });
          }
        } else {
          this.disposeMaterial(litMaterial as { map?: { dispose?: () => void }; dispose?: () => void });
        }
      }
    });
  }

  private disposeMaterial(material: { map?: { dispose?: () => void }; dispose?: () => void }): void {
    material.map?.dispose?.();
    material.dispose?.();
  }
}
