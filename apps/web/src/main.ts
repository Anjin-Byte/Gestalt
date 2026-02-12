/**
 * Application entry point.
 *
 * Bootstraps the testbed: initializes the viewer, module system, and UI panels.
 */

import "./style.css";
import { createDefaultModules } from "./modules/registry";
import { ModuleHost } from "./modules/moduleHost";
import { createThreeBackend } from "./viewer/threeBackend";
import { Viewer } from "./viewer/Viewer";
import { initDebugOverlay } from "./ui/debugOverlay";
import { buildDebugPanel } from "./ui/debugPanel";
import { buildScenePanel } from "./ui/scenePanel";
import { AnimationLoop } from "./ui/animationLoop";

const app = async () => {
  // === DOM Elements ===
  const canvas = document.getElementById("viewport") as HTMLCanvasElement | null;
  const overlay = document.getElementById("overlay") as HTMLElement | null;
  const modulePanel = document.getElementById("module-panel") as HTMLElement | null;
  const scenePanel = document.getElementById("scene-panel") as HTMLElement | null;
  const debugPanel = document.getElementById("debug-panel") as HTMLElement | null;
  const statusText = document.getElementById("status-text") as HTMLElement | null;

  if (!canvas || !overlay || !modulePanel || !scenePanel || !debugPanel || !statusText) {
    throw new Error("Missing required DOM elements.");
  }

  // === Configuration ===
  const logger = {
    info: (message: string) => console.info(message),
    warn: (message: string) => console.warn(message),
    error: (message: string) => console.error(message),
  };

  const params = new URLSearchParams(window.location.search);
  const testMode = params.get("test") === "1";
  const savedPreference =
    (localStorage.getItem("rendererPreference") as "auto" | "webgpu" | "webgl" | null) ?? "auto";

  document.body.classList.add("viewport-full");

  // === Viewer Setup ===
  const backend = await createThreeBackend(canvas, {
    testMode,
    preferredRenderer: savedPreference,
  });
  const viewer = new Viewer(backend, { overlay, testMode });

  const settingsOverlay = document.createElement("div");
  settingsOverlay.id = "settings-overlay";
  settingsOverlay.className = "settings-overlay";
  settingsOverlay.innerHTML = `
    <div class="settings-overlay__panel">
      <div class="settings-overlay__header">
        <span>Settings</span>
        <span class="settings-overlay__hint">Press E to close</span>
      </div>
      <div class="settings-overlay__body"></div>
    </div>
  `;
  const viewport = canvas.parentElement;
  if (viewport) {
    viewport.appendChild(settingsOverlay);
  }

  const settingsBody = settingsOverlay.querySelector(
    ".settings-overlay__body"
  ) as HTMLElement | null;
  if (!settingsBody) {
    throw new Error("Settings overlay body missing.");
  }

  let settingsOpen = false;
  const setSettingsOpen = (open: boolean) => {
    settingsOpen = open;
    settingsOverlay.dataset.open = open ? "true" : "false";
    settingsOverlay.style.display = open ? "flex" : "none";
    backend.controls.setEnabled(!open);
  };
  setSettingsOpen(false);

  const isTypingTarget = (target: EventTarget | null) => {
    if (!(target instanceof HTMLElement)) return false;
    const tag = target.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
  };

  window.addEventListener("keydown", (event) => {
    if (event.code !== "KeyE") {
      return;
    }
    if (isTypingTarget(event.target)) {
      return;
    }
    event.preventDefault();
    setSettingsOpen(!settingsOpen);
  });

  // Initialize debug overlay
  if (viewport) {
    initDebugOverlay({ container: viewport, visible: true });
  }

  // === Resolution Management ===
  let lockResolution = false;
  let lockedWidth = 960;
  let lockedHeight = 540;

  const resize = () => {
    if (lockResolution) {
      canvas.style.width = `${lockedWidth}px`;
      canvas.style.height = `${lockedHeight}px`;
      viewer.resize(lockedWidth, lockedHeight);
    } else {
      const rect = canvas.getBoundingClientRect();
      canvas.style.width = "100%";
      canvas.style.height = "100%";
      viewer.resize(rect.width, rect.height);
    }
  };

  window.addEventListener("resize", resize);
  resize();

  // === GPU Device ===
  let gpuDevicePromise: Promise<GPUDevice | null> | null = null;
  const requestGpuDevice = async (): Promise<GPUDevice | null> => {
    if (!("gpu" in navigator)) return null;
    if (!gpuDevicePromise) {
      gpuDevicePromise = navigator.gpu
        .requestAdapter()
        .then((adapter: GPUAdapter | null) => (adapter ? adapter.requestDevice() : null));
    }
    return gpuDevicePromise;
  };

  // === Animation Loop ===
  const animationLoop = new AnimationLoop({
    render: () => viewer.render(),
    statusElement: statusText,
  });

  // === Scene Panel ===
  buildScenePanel({ container: scenePanel, viewer });

  // === Debug Panel ===
  buildDebugPanel({
    container: debugPanel,
    viewer,
    backend,
    logger,
    savedRendererPreference: savedPreference,
    requestGpuDevice,
    onResolutionChange: (locked, width, height) => {
      lockResolution = locked;
      lockedWidth = width;
      lockedHeight = height;
      resize();
    },
    onFpsChange: (fps) => {
      animationLoop.setTargetFps(fps);
    },
  });

  // === Module System ===
  const ctx = { requestGpuDevice, logger, baseUrl: import.meta.env.BASE_URL };

  const moduleSection = document.createElement("section");
  moduleSection.className = "settings-overlay__section";
  const moduleSectionTitle = document.createElement("div");
  moduleSectionTitle.className = "settings-overlay__section-title";
  moduleSectionTitle.textContent = "Module";
  moduleSection.appendChild(moduleSectionTitle);

  const moduleSelectLabel = document.createElement("label");
  moduleSelectLabel.textContent = "Active Module";
  moduleSection.appendChild(moduleSelectLabel);

  const moduleSelect = document.createElement("select");
  moduleSelect.dataset.testid = "module-select";
  moduleSection.appendChild(moduleSelect);

  const runButton = document.createElement("button");
  runButton.textContent = "Run Module";
  runButton.dataset.testid = "run-module";
  moduleSection.appendChild(runButton);

  const moduleControls = document.createElement("div");
  moduleControls.id = "module-controls";
  moduleSection.appendChild(moduleControls);

  settingsBody.appendChild(moduleSection);

  const modules = createDefaultModules();
  const host = new ModuleHost(modules, moduleControls, ctx, (outputs) => {
    viewer.setOutputs(outputs);
    if (testMode) viewer.render();
  });

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

  window.addEventListener("beforeunload", () => {
    void host.dispose();
  });

  runButton.addEventListener("click", () => {
    host.runActive();
  });

  // === Start ===
  if (testMode) {
    backend.controls.enableRotate = false;
    backend.controls.enablePan = false;
    backend.controls.enableZoom = false;
    viewer.render();
    document.body.dataset.ready = "true";
  } else {
    animationLoop.start();
  }
};

app();
