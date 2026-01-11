import {
  ACESFilmicToneMapping,
  AmbientLight,
  DirectionalLight,
  HemisphereLight,
  PerspectiveCamera,
  Scene,
  SRGBColorSpace,
  WebGLRenderer
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import WebGPURenderer from "three/src/renderers/webgpu/WebGPURenderer.js";

export type BackendOptions = {
  testMode: boolean;
  preferredRenderer: "auto" | "webgpu" | "webgl";
};

export type ViewerBackend = {
  renderer: WebGLRenderer | WebGPURenderer;
  scene: Scene;
  camera: PerspectiveCamera;
  controls: OrbitControls;
  isWebGPU: boolean;
  resize: (width: number, height: number) => void;
  render: () => void;
  dispose: () => void;
  rebindControls: (domElement: HTMLElement) => void;
  setExposure: (value: number) => void;
  setLightScale: (value: number) => void;
  getExposure: () => number;
  getLightScale: () => number;
};

export const createThreeBackend = async (
  canvas: HTMLCanvasElement,
  options: BackendOptions
): Promise<ViewerBackend> => {
  let renderer: WebGLRenderer | WebGPURenderer | null = null;
  let isWebGPU = false;

  if (options.preferredRenderer !== "webgl" && "gpu" in navigator) {
    try {
      const webgpu = new WebGPURenderer({ canvas, antialias: true });
      await webgpu.init();
      renderer = webgpu;
      isWebGPU = true;
    } catch (error) {
      console.warn("WebGPU init failed, falling back to WebGL.", error);
    }
  }

  if (!renderer) {
    renderer = new WebGLRenderer({ canvas, antialias: true });
  }

  renderer.outputColorSpace = SRGBColorSpace;
  renderer.toneMapping = ACESFilmicToneMapping;
  renderer.toneMappingExposure = isWebGPU ? 1.4 : 1.0;

  renderer.setPixelRatio(options.testMode ? 1 : window.devicePixelRatio);

  const scene = new Scene();
  const camera = new PerspectiveCamera(60, 1, 0.1, 500);
  camera.position.set(4, 4, 6);

  let controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = !options.testMode;
  controls.target.set(0, 0, 0);
  controls.update();

  const baseLights = {
    ambient: 0.7,
    hemi: 0.6,
    sun: 1.8
  };
  let lightScale = isWebGPU ? 1.5 : 1.0;
  const ambient = new AmbientLight(0xffffff, baseLights.ambient * lightScale);
  scene.add(ambient);
  const hemi = new HemisphereLight(
    0x8fd3ff,
    0x1a1e2a,
    baseLights.hemi * lightScale
  );
  scene.add(hemi);
  const sun = new DirectionalLight(0xffffff, baseLights.sun * lightScale);
  sun.position.set(6, 8, 4);
  scene.add(sun);

  const resize = (width: number, height: number) => {
    renderer?.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  };

  return {
    renderer,
    scene,
    camera,
    controls,
    isWebGPU,
    resize,
    render: () => {
      controls.update();
      renderer?.render(scene, camera);
    },
    dispose: () => {
      controls.dispose();
      renderer?.dispose();
    },
    rebindControls: (domElement: HTMLElement) => {
      const target = controls.target.clone();
      const damping = controls.enableDamping;
      const enablePan = controls.enablePan;
      const enableZoom = controls.enableZoom;
      const enableRotate = controls.enableRotate;
      controls.dispose();
      controls = new OrbitControls(camera, domElement);
      controls.enableDamping = damping;
      controls.enablePan = enablePan;
      controls.enableZoom = enableZoom;
      controls.enableRotate = enableRotate;
      controls.target.copy(target);
      controls.update();
    },
    setExposure: (value: number) => {
      renderer.toneMappingExposure = value;
    },
    setLightScale: (value: number) => {
      lightScale = value;
      ambient.intensity = baseLights.ambient * lightScale;
      hemi.intensity = baseLights.hemi * lightScale;
      sun.intensity = baseLights.sun * lightScale;
    },
    getExposure: () => renderer.toneMappingExposure,
    getLightScale: () => lightScale
  };
};
