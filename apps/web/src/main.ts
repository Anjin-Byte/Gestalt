import "./style.css";
import { createDefaultModules } from "./modules/registry";
import { ModuleHost } from "./modules/moduleHost";
import { createThreeBackend } from "./viewer/threeBackend";
import { attachWebgpuErrorLogger } from "./viewer/webgpuDiagnostics";
import { Viewer } from "./viewer/Viewer";
import { initDebugOverlay, getDebugOverlay } from "./ui/debugOverlay";

const app = async () => {
  const canvas = document.getElementById("viewport") as HTMLCanvasElement | null;
  if (!canvas) {
    throw new Error("Viewport canvas missing.");
  }
  const overlay = document.getElementById("overlay") as HTMLElement | null;
  if (!overlay) {
    throw new Error("Overlay element missing.");
  }

  const modulePanel = document.getElementById("module-panel") as HTMLElement | null;
  const scenePanel = document.getElementById("scene-panel") as HTMLElement | null;
  const debugPanel = document.getElementById("debug-panel") as HTMLElement | null;
  const statusText = document.getElementById("status-text") as HTMLElement | null;

  if (!modulePanel || !scenePanel || !debugPanel || !statusText) {
    throw new Error("Missing UI panels.");
  }

  const logger = {
    info: (message: string) => console.info(message),
    warn: (message: string) => console.warn(message),
    error: (message: string) => console.error(message)
  };

  const params = new URLSearchParams(window.location.search);
  const testMode = params.get("test") === "1";
  const savedPreference =
    (localStorage.getItem("rendererPreference") as
      | "auto"
      | "webgpu"
      | "webgl"
      | null) ?? "auto";

  const backend = await createThreeBackend(canvas, {
    testMode,
    preferredRenderer: savedPreference
  });
  const viewer = new Viewer(backend, { overlay, testMode });

  // Initialize debug overlay on viewport
  const viewport = canvas.parentElement;
  if (viewport) {
    initDebugOverlay({ container: viewport, visible: true });
  }

  let lockResolution = false;
  let lockedWidth = 960;
  let lockedHeight = 540;
  const applyCanvasSize = (width: number, height: number) => {
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    viewer.resize(width, height);
  };

  const resize = () => {
    if (lockResolution) {
      applyCanvasSize(lockedWidth, lockedHeight);
      return;
    }
    const rect = canvas.getBoundingClientRect();
    canvas.style.width = "100%";
    canvas.style.height = "100%";
    viewer.resize(rect.width, rect.height);
  };
  window.addEventListener("resize", resize);
  resize();

  let gpuDevicePromise: Promise<GPUDevice | null> | null = null;
  const ctx = {
    requestGpuDevice: async () => {
      if (!("gpu" in navigator)) {
        return null;
      }
      if (!gpuDevicePromise) {
        gpuDevicePromise = navigator.gpu
          .requestAdapter()
          .then((adapter: GPUAdapter | null) =>
            adapter ? adapter.requestDevice() : null
          );
      }
      return gpuDevicePromise;
    },
    logger,
    baseUrl: import.meta.env.BASE_URL,
  };

  const moduleSelectLabel = document.createElement("label");
  moduleSelectLabel.textContent = "Active Module";
  modulePanel.appendChild(moduleSelectLabel);

  const moduleSelect = document.createElement("select");
  moduleSelect.dataset.testid = "module-select";
  modulePanel.appendChild(moduleSelect);

  const runButton = document.createElement("button");
  runButton.textContent = "Run Module";
  runButton.dataset.testid = "run-module";
  modulePanel.appendChild(runButton);

  const moduleControls = document.createElement("div");
  moduleControls.id = "module-controls";
  modulePanel.appendChild(moduleControls);

  const modules = createDefaultModules();
  const host = new ModuleHost(modules, moduleControls, ctx, (outputs) => {
    viewer.setOutputs(outputs);
    if (testMode) {
      viewer.render();
    }
  });
  await host.initAll();

  for (const module of host.list()) {
    const option = document.createElement("option");
    option.value = module.id;
    option.textContent = module.name;
    moduleSelect.appendChild(option);
  }

  const activateModule = async () => {
    await host.activate(moduleSelect.value);
  };
  moduleSelect.addEventListener("change", activateModule);
  await activateModule();

  runButton.addEventListener("click", () => {
    host.runActive();
  });

  const wireframeToggle = document.createElement("input");
  wireframeToggle.type = "checkbox";
  wireframeToggle.addEventListener("change", () => {
    viewer.setWireframe(wireframeToggle.checked);
  });
  const wireframeLabel = document.createElement("label");
  wireframeLabel.textContent = "Wireframe";
  wireframeLabel.prepend(wireframeToggle);
  scenePanel.appendChild(wireframeLabel);

  const frameButton = document.createElement("button");
  frameButton.textContent = "Frame Object";
  frameButton.addEventListener("click", () => {
    viewer.frameObject();
  });
  scenePanel.appendChild(frameButton);

  const gridToggle = document.createElement("input");
  gridToggle.type = "checkbox";
  gridToggle.checked = false;
  gridToggle.addEventListener("change", () => {
    viewer.setGridVisible(gridToggle.checked);
  });
  const gridLabel = document.createElement("label");
  gridLabel.textContent = "Grid";
  gridLabel.prepend(gridToggle);
  debugPanel.appendChild(gridLabel);

  const axesToggle = document.createElement("input");
  axesToggle.type = "checkbox";
  axesToggle.checked = true;
  axesToggle.addEventListener("change", () => {
    viewer.setAxesVisible(axesToggle.checked);
  });
  const axesLabel = document.createElement("label");
  axesLabel.textContent = "Axes";
  axesLabel.prepend(axesToggle);
  debugPanel.appendChild(axesLabel);

  const boundsToggle = document.createElement("input");
  boundsToggle.type = "checkbox";
  boundsToggle.checked = false;
  boundsToggle.addEventListener("change", () => {
    viewer.setBoundsVisible(boundsToggle.checked);
  });
  const boundsLabel = document.createElement("label");
  boundsLabel.textContent = "Bounds";
  boundsLabel.prepend(boundsToggle);
  debugPanel.appendChild(boundsLabel);
  viewer.setGridVisible(false);
  viewer.setBoundsVisible(false);

  const statsOverlayToggle = document.createElement("input");
  statsOverlayToggle.type = "checkbox";
  statsOverlayToggle.checked = true;
  statsOverlayToggle.addEventListener("change", () => {
    const debugOverlay = getDebugOverlay();
    debugOverlay?.setVisible(statsOverlayToggle.checked);
  });
  const statsOverlayLabel = document.createElement("label");
  statsOverlayLabel.textContent = "Stats Overlay";
  statsOverlayLabel.prepend(statsOverlayToggle);
  debugPanel.appendChild(statsOverlayLabel);

  const unlitToggle = document.createElement("input");
  unlitToggle.type = "checkbox";
  unlitToggle.addEventListener("change", () => {
    viewer.setUnlit(unlitToggle.checked);
  });
  const unlitLabel = document.createElement("label");
  unlitLabel.textContent = "Unlit Material";
  unlitLabel.prepend(unlitToggle);
  debugPanel.appendChild(unlitLabel);

  const backendInfo = document.createElement("div");
  backendInfo.textContent = backend.isWebGPU ? "Renderer: WebGPU" : "Renderer: WebGL2";
  debugPanel.appendChild(backendInfo);

  const limitsInfo = document.createElement("div");
  limitsInfo.textContent = "Device Limits: (querying...)";
  debugPanel.appendChild(limitsInfo);
  ctx.requestGpuDevice().then((device) => {
    if (!device) {
      limitsInfo.textContent = "Device Limits: WebGPU unavailable";
      return;
    }
    const limits = device.limits;
    limitsInfo.textContent =
      "Device Limits: " +
      `maxInvocations=${limits.maxComputeInvocationsPerWorkgroup} ` +
      `maxStorageMB=${Math.round(limits.maxStorageBufferBindingSize / (1024 * 1024))}`;
  });

  const exposureLabel = document.createElement("label");
  exposureLabel.textContent = "Exposure";
  debugPanel.appendChild(exposureLabel);
  const exposureSlider = document.createElement("input");
  exposureSlider.type = "range";
  exposureSlider.min = "0.6";
  exposureSlider.max = "2.5";
  exposureSlider.step = "0.05";
  exposureSlider.value = String(backend.getExposure());
  exposureSlider.addEventListener("input", () => {
    backend.setExposure(Number(exposureSlider.value));
  });
  debugPanel.appendChild(exposureSlider);

  const lightLabel = document.createElement("label");
  lightLabel.textContent = "Light Scale";
  debugPanel.appendChild(lightLabel);
  const lightSlider = document.createElement("input");
  lightSlider.type = "range";
  lightSlider.min = "0.2";
  lightSlider.max = "3.0";
  lightSlider.step = "0.1";
  lightSlider.value = String(backend.getLightScale());
  lightSlider.addEventListener("input", () => {
    backend.setLightScale(Number(lightSlider.value));
  });
  debugPanel.appendChild(lightSlider);

  const webgpuLogToggle = document.createElement("input");
  webgpuLogToggle.type = "checkbox";
  let webgpuLogger: { detach: () => void; hasDevice: boolean } = {
    detach: () => undefined,
    hasDevice: false
  };
  webgpuLogToggle.checked = false;
  webgpuLogToggle.addEventListener("change", () => {
    if (webgpuLogToggle.checked) {
      webgpuLogger = attachWebgpuErrorLogger(backend.renderer, logger);
      if (!webgpuLogger.hasDevice) {
        logger.warn("WebGPU error logging unavailable: no device.");
      }
    } else {
      webgpuLogger.detach();
    }
  });
  const webgpuLogLabel = document.createElement("label");
  webgpuLogLabel.textContent = "Log WebGPU Errors";
  webgpuLogLabel.prepend(webgpuLogToggle);
  debugPanel.appendChild(webgpuLogLabel);

  const rendererLabel = document.createElement("label");
  rendererLabel.textContent = "Renderer Preference";
  debugPanel.appendChild(rendererLabel);

  const rendererSelect = document.createElement("select");
  rendererSelect.dataset.testid = "renderer-select";
  const options = [
    { label: "Auto", value: "auto" },
    { label: "WebGPU", value: "webgpu" },
    { label: "WebGL2", value: "webgl" }
  ];
  for (const option of options) {
    const item = document.createElement("option");
    item.value = option.value;
    item.textContent = option.label;
    rendererSelect.appendChild(item);
  }
  rendererSelect.value = savedPreference;
  rendererSelect.addEventListener("change", () => {
    localStorage.setItem("rendererPreference", rendererSelect.value);
    window.location.reload();
  });
  debugPanel.appendChild(rendererSelect);

  const fpsLabel = document.createElement("label");
  fpsLabel.textContent = "Frame Rate";
  debugPanel.appendChild(fpsLabel);

  const fpsSelect = document.createElement("select");
  const fpsOptions = [
    { label: "Uncapped", value: "0" },
    { label: "30 FPS", value: "30" },
    { label: "60 FPS", value: "60" },
    { label: "120 FPS", value: "120" }
  ];
  for (const option of fpsOptions) {
    const item = document.createElement("option");
    item.value = option.value;
    item.textContent = option.label;
    fpsSelect.appendChild(item);
  }
  fpsSelect.value = "0";
  debugPanel.appendChild(fpsSelect);

  const lockToggle = document.createElement("input");
  lockToggle.type = "checkbox";
  lockToggle.addEventListener("change", () => {
    lockResolution = lockToggle.checked;
    resize();
  });
  const lockLabel = document.createElement("label");
  lockLabel.textContent = "Lock Render Size";
  lockLabel.prepend(lockToggle);
  debugPanel.appendChild(lockLabel);

  const lockWidthInput = document.createElement("input");
  lockWidthInput.type = "number";
  lockWidthInput.min = "320";
  lockWidthInput.max = "3840";
  lockWidthInput.value = String(lockedWidth);
  lockWidthInput.addEventListener("change", () => {
    lockedWidth = Math.max(320, Number(lockWidthInput.value));
    if (lockResolution) {
      resize();
    }
  });
  debugPanel.appendChild(lockWidthInput);

  const lockHeightInput = document.createElement("input");
  lockHeightInput.type = "number";
  lockHeightInput.min = "240";
  lockHeightInput.max = "2160";
  lockHeightInput.value = String(lockedHeight);
  lockHeightInput.addEventListener("change", () => {
    lockedHeight = Math.max(240, Number(lockHeightInput.value));
    if (lockResolution) {
      resize();
    }
  });
  debugPanel.appendChild(lockHeightInput);

  let lastSample = performance.now();
  let frames = 0;
  let targetFps = 0;
  let lastFrameTime = performance.now();
  fpsSelect.addEventListener("change", () => {
    targetFps = Number(fpsSelect.value);
    lastFrameTime = performance.now();
  });

  const updateStatus = () => {
    const now = performance.now();
    frames += 1;
    const delta = now - lastSample;
    if (delta >= 500) {
      const fps = Math.round((frames / delta) * 1000);
      statusText.textContent = `FPS: ${fps} | Frame: ${(delta / frames).toFixed(1)} ms`;
      lastSample = now;
      frames = 0;
    }
  };

  const animate = () => {
    const now = performance.now();
    if (targetFps > 0) {
      const frameDuration = 1000 / targetFps;
      if (now - lastFrameTime < frameDuration) {
        requestAnimationFrame(animate);
        return;
      }
      lastFrameTime = now;
    }
    viewer.render();
    updateStatus();
    requestAnimationFrame(animate);
  };

  if (testMode) {
    backend.controls.enableRotate = false;
    backend.controls.enablePan = false;
    backend.controls.enableZoom = false;
    viewer.render();
    document.body.dataset.ready = "true";
  } else {
    animate();
  }
};

app();
