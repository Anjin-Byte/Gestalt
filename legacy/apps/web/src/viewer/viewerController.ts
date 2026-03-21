import type { ModuleOutput, Logger } from "../modules/types";
import type { ViewerBackend } from "./threeBackend";
import { createThreeBackend } from "./threeBackend";
import { Viewer } from "./Viewer";

export type RendererPreference = "auto" | "webgpu" | "webgl";

export type ViewerControllerOptions = {
  testMode: boolean;
  logger: Logger;
};

export class ViewerController {
  private backend: ViewerBackend | null = null;
  private viewer: Viewer | null = null;
  private lastOutputs: ModuleOutput[] = [];
  private lastSize = { width: 1, height: 1 };
  private wireframe = false;
  private unlit = false;
  private gridVisible = false;
  private axesVisible = true;
  private boundsVisible = false;
  private exposure = 1.0;
  private lightScale = 1.0;

  constructor(
    private canvas: HTMLCanvasElement,
    private readonly overlay: HTMLElement,
    private readonly options: ViewerControllerOptions
  ) {}

  async init(preference: RendererPreference): Promise<void> {
    await this.createBackend(preference);
  }

  async switchRenderer(
    preference: RendererPreference,
    canvasOverride?: HTMLCanvasElement
  ): Promise<void> {
    if (canvasOverride) {
      this.canvas = canvasOverride;
    }
    await this.createBackend(preference, true);
  }

  get renderer(): ViewerBackend["renderer"] | null {
    return this.backend?.renderer ?? null;
  }

  getCanvas(): HTMLCanvasElement {
    return this.canvas;
  }

  get isWebGPU(): boolean {
    return this.backend?.isWebGPU ?? false;
  }

  setOutputs(outputs: ModuleOutput[]): void {
    this.lastOutputs = outputs;
    this.viewer?.setOutputs(outputs);
  }

  setWireframe(enabled: boolean): void {
    this.wireframe = enabled;
    this.viewer?.setWireframe(enabled);
  }

  setUnlit(enabled: boolean): void {
    this.unlit = enabled;
    this.viewer?.setUnlit(enabled);
  }

  setGridVisible(visible: boolean): void {
    this.gridVisible = visible;
    this.viewer?.setGridVisible(visible);
  }

  setAxesVisible(visible: boolean): void {
    this.axesVisible = visible;
    this.viewer?.setAxesVisible(visible);
  }

  setBoundsVisible(visible: boolean): void {
    this.boundsVisible = visible;
    this.viewer?.setBoundsVisible(visible);
  }

  frameObject(): void {
    this.viewer?.frameObject();
  }

  render(): void {
    this.backend?.render();
  }

  resize(width: number, height: number): void {
    this.lastSize = { width, height };
    this.backend?.resize(width, height);
  }

  setExposure(value: number): void {
    this.exposure = value;
    this.backend?.setExposure(value);
  }

  setLightScale(value: number): void {
    this.lightScale = value;
    this.backend?.setLightScale(value);
  }

  getExposure(): number {
    return this.backend?.getExposure() ?? this.exposure;
  }

  getLightScale(): number {
    return this.backend?.getLightScale() ?? this.lightScale;
  }

  private async createBackend(
    preference: RendererPreference,
    disposePrevious = false
  ): Promise<void> {
    const previous = this.backend;

    const backend = await createThreeBackend(this.canvas, {
      testMode: this.options.testMode,
      preferredRenderer: preference
    });

    const viewer = new Viewer(backend, {
      overlay: this.overlay,
      testMode: this.options.testMode
    });

    this.backend = backend;
    this.viewer = viewer;

    if (disposePrevious) {
      previous?.dispose();
    }

    if (this.options.testMode) {
      backend.controls.enableRotate = false;
      backend.controls.enablePan = false;
      backend.controls.enableZoom = false;
    }

    viewer.setWireframe(this.wireframe);
    viewer.setUnlit(this.unlit);
    viewer.setGridVisible(this.gridVisible);
    viewer.setAxesVisible(this.axesVisible);
    viewer.setBoundsVisible(this.boundsVisible);
    backend.setExposure(this.exposure);
    backend.setLightScale(this.lightScale);

    viewer.setOutputs(this.lastOutputs);
    backend.resize(this.lastSize.width, this.lastSize.height);

    viewer.render();
  }

}
