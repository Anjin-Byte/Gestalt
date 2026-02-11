/**
 * Debug panel UI construction.
 *
 * Builds all controls for the debug panel: visualization toggles,
 * renderer settings, performance options, and resolution controls.
 */

import type { Viewer } from "../viewer/Viewer";
import type { ViewerBackend } from "../viewer/threeBackend";
import { attachWebgpuErrorLogger } from "../viewer/webgpuDiagnostics";
import { getDebugOverlay } from "./debugOverlay";

export type Logger = {
  info: (message: string) => void;
  warn: (message: string) => void;
  error: (message: string) => void;
};

export type DebugPanelOptions = {
  container: HTMLElement;
  viewer: Viewer;
  backend: ViewerBackend;
  logger: Logger;
  savedRendererPreference: "auto" | "webgpu" | "webgl";
  requestGpuDevice: () => Promise<GPUDevice | null>;
  onResolutionChange: (locked: boolean, width: number, height: number) => void;
  onFpsChange: (targetFps: number) => void;
};

/** Build the debug panel UI and attach to the container. */
export function buildDebugPanel(options: DebugPanelOptions): void {
  const { container, viewer, backend, logger } = options;

  // === Visualization Toggles ===
  appendCheckbox(container, "Grid", false, (checked) => {
    viewer.setGridVisible(checked);
  });

  appendCheckbox(container, "Axes", true, (checked) => {
    viewer.setAxesVisible(checked);
  });

  appendCheckbox(container, "Bounds", false, (checked) => {
    viewer.setBoundsVisible(checked);
  });

  appendCheckbox(container, "Stats Overlay", true, (checked) => {
    getDebugOverlay()?.setVisible(checked);
  });

  appendCheckbox(container, "Unlit Material", false, (checked) => {
    viewer.setUnlit(checked);
  });

  // === Renderer Info ===
  appendInfo(container, backend.isWebGPU ? "Renderer: WebGPU" : "Renderer: WebGL2");

  const limitsInfo = appendInfo(container, "Device Limits: (querying...)");
  options.requestGpuDevice().then((device) => {
    if (!device) {
      limitsInfo.textContent = "Device Limits: WebGPU unavailable";
      return;
    }
    const limits = device.limits;
    limitsInfo.textContent =
      `Device Limits: maxInvocations=${limits.maxComputeInvocationsPerWorkgroup} ` +
      `maxStorageMB=${Math.round(limits.maxStorageBufferBindingSize / (1024 * 1024))}`;
  });

  // === Lighting Controls ===
  appendSlider(container, "Exposure", 0.6, 2.5, 0.05, backend.getExposure(), (value) => {
    backend.setExposure(value);
  });

  appendSlider(container, "Light Scale", 0.2, 3.0, 0.1, backend.getLightScale(), (value) => {
    backend.setLightScale(value);
  });

  // === WebGPU Logging ===
  let webgpuLogger: { detach: () => void; hasDevice: boolean } = {
    detach: () => undefined,
    hasDevice: false,
  };

  appendCheckbox(container, "Log WebGPU Errors", false, (checked) => {
    if (checked) {
      webgpuLogger = attachWebgpuErrorLogger(backend.renderer, logger);
      if (!webgpuLogger.hasDevice) {
        logger.warn("WebGPU error logging unavailable: no device.");
      }
    } else {
      webgpuLogger.detach();
    }
  });

  // === Renderer Preference ===
  appendSelect(
    container,
    "Renderer Preference",
    [
      { label: "Auto", value: "auto" },
      { label: "WebGPU", value: "webgpu" },
      { label: "WebGL2", value: "webgl" },
    ],
    options.savedRendererPreference,
    (value) => {
      localStorage.setItem("rendererPreference", value);
      window.location.reload();
    },
    "renderer-select"
  );

  // === Frame Rate ===
  appendSelect(
    container,
    "Frame Rate",
    [
      { label: "Uncapped", value: "0" },
      { label: "30 FPS", value: "30" },
      { label: "60 FPS", value: "60" },
      { label: "120 FPS", value: "120" },
    ],
    "0",
    (value) => {
      options.onFpsChange(Number(value));
    }
  );

  // === Resolution Lock ===
  let lockResolution = false;
  let lockedWidth = 960;
  let lockedHeight = 540;

  const notifyResolution = () => {
    options.onResolutionChange(lockResolution, lockedWidth, lockedHeight);
  };

  appendCheckbox(container, "Lock Render Size", false, (checked) => {
    lockResolution = checked;
    notifyResolution();
  });

  appendNumberInput(container, 320, 3840, lockedWidth, (value) => {
    lockedWidth = Math.max(320, value);
    if (lockResolution) notifyResolution();
  });

  appendNumberInput(container, 240, 2160, lockedHeight, (value) => {
    lockedHeight = Math.max(240, value);
    if (lockResolution) notifyResolution();
  });

  // Apply initial state
  viewer.setGridVisible(false);
  viewer.setBoundsVisible(false);
}

// === Helper Functions ===

function appendCheckbox(
  container: HTMLElement,
  labelText: string,
  initial: boolean,
  onChange: (checked: boolean) => void
): HTMLInputElement {
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = initial;
  input.addEventListener("change", () => onChange(input.checked));

  const label = document.createElement("label");
  label.textContent = labelText;
  label.prepend(input);
  container.appendChild(label);

  return input;
}

function appendSlider(
  container: HTMLElement,
  labelText: string,
  min: number,
  max: number,
  step: number,
  initial: number,
  onChange: (value: number) => void
): HTMLInputElement {
  const label = document.createElement("label");
  label.textContent = labelText;
  container.appendChild(label);

  const input = document.createElement("input");
  input.type = "range";
  input.min = String(min);
  input.max = String(max);
  input.step = String(step);
  input.value = String(initial);
  input.addEventListener("input", () => onChange(Number(input.value)));
  container.appendChild(input);

  return input;
}

function appendSelect(
  container: HTMLElement,
  labelText: string,
  options: Array<{ label: string; value: string }>,
  initial: string,
  onChange: (value: string) => void,
  testId?: string
): HTMLSelectElement {
  const label = document.createElement("label");
  label.textContent = labelText;
  container.appendChild(label);

  const select = document.createElement("select");
  if (testId) select.dataset.testid = testId;

  for (const opt of options) {
    const item = document.createElement("option");
    item.value = opt.value;
    item.textContent = opt.label;
    select.appendChild(item);
  }

  select.value = initial;
  select.addEventListener("change", () => onChange(select.value));
  container.appendChild(select);

  return select;
}

function appendNumberInput(
  container: HTMLElement,
  min: number,
  max: number,
  initial: number,
  onChange: (value: number) => void
): HTMLInputElement {
  const input = document.createElement("input");
  input.type = "number";
  input.min = String(min);
  input.max = String(max);
  input.value = String(initial);
  input.addEventListener("change", () => onChange(Number(input.value)));
  container.appendChild(input);

  return input;
}

function appendInfo(container: HTMLElement, text: string): HTMLDivElement {
  const div = document.createElement("div");
  div.textContent = text;
  container.appendChild(div);
  return div;
}
