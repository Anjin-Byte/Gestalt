// The GUI shell: DOM, HUD, input forwarding, file pickers, downloads — and
// nothing else. Rendering lives in the render worker (or the main-thread
// fallback host) and every scene build/codec runs in the IO worker; the shell
// routes transferable blobs between them
// (docs/design/web-frontend-api.md §3–§5). Everything here is importable
// without side effects — main.ts is the entry that actually boots.
import { BrushTool, CameraMode, Falloff, MeshFormat } from "voxel-web";

import { type Demo, fetchDemoMesh, listDemos } from "./demos";
import { Hud } from "./hud";
import { attachInput } from "./input";
import { IoClient, type IoJobs } from "./io";
import {
  DECODE_PHASES,
  ENCODE_PHASES,
  MESH_PHASES,
  importKindFor,
  type JobProgress,
  type ProgressPhase,
  type VoxelizeOptions,
} from "./io-protocol";
import { BuildBar } from "./progress-bar";
import { LocalRenderHost, WorkerRenderHost, type RenderHost } from "./render";
import { MAX_DPR, type SceneMeta } from "./render-protocol";

/** The initial scene — a gyroid backdrop behind the demo gallery: sleek,
 * space-filling, and instant (a procedural CPU fixture, no network). Resolutions
 * are the structure's legal 8·4^k sizes; 128 builds fast (and off the main
 * thread anyway). */
const DEFAULT_OPTIONS = { res: 128, fixture: "gyroid" };

/** A DOM invariant failure — the page markup is the shell's own contract, so a
 * missing element is process-fatal, not recoverable. */
export function must<T extends Element>(root: Document, selector: string, kind: new () => T): T {
  const node = root.querySelector(selector);
  if (!(node instanceof kind)) {
    throw new Error(`missing/mistyped element ${selector}`);
  }
  return node;
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export interface Ui {
  canvas: HTMLCanvasElement;
  // The dock toggles and their panels: each surface opens/closes on its own
  // button (state = aria-expanded, tinted by CSS), persisted per panel.
  readonly toggleEdit: HTMLButtonElement;
  readonly toggleScene: HTMLButtonElement;
  readonly toggleIo: HTMLButtonElement;
  readonly toggleStats: HTMLButtonElement;
  readonly panelEdit: HTMLElement;
  /** The theme override (sun/moon): follows the OS until clicked. */
  readonly themeToggle: HTMLButtonElement;
  readonly panelScene: HTMLElement;
  readonly panelIo: HTMLElement;
  readonly panelStats: HTMLElement;
  readonly toggleSettings: HTMLButtonElement;
  readonly panelSettings: HTMLElement;
  /** The accent swatches, in the settings panel. */
  readonly accentSwatches: readonly HTMLButtonElement[];
  /** Opens the demo gallery (which also opens at boot). */
  readonly browseDemos: HTMLButtonElement;
  /** Read-only name of the current scene (replaced the fixture picker's status role). */
  readonly sceneName: HTMLElement;
  /** The gallery launcher overlay and its card grid / empty state. */
  readonly gallery: HTMLElement;
  readonly galleryGrid: HTMLElement;
  readonly galleryEmpty: HTMLElement;
  readonly res: HTMLSelectElement;
  readonly truecolor: HTMLInputElement;
  readonly zup: HTMLInputElement;
  readonly gpuBake: HTMLInputElement;
  readonly toolDraw: HTMLButtonElement;
  readonly toolErase: HTMLButtonElement;
  readonly toolClay: HTMLButtonElement;
  readonly toolSmooth: HTMLButtonElement;
  readonly toolFlatten: HTMLButtonElement;
  readonly toolInflate: HTMLButtonElement;
  readonly toolPaint: HTMLButtonElement;
  readonly brushRadius: HTMLInputElement;
  readonly radiusOut: HTMLOutputElement;
  readonly strength: HTMLInputElement;
  readonly strengthOut: HTMLOutputElement;
  readonly falloffSmooth: HTMLButtonElement;
  readonly falloffLinear: HTMLButtonElement;
  readonly falloffSphere: HTMLButtonElement;
  readonly falloffSharp: HTMLButtonElement;
  readonly brushColor: HTMLInputElement;
  readonly undoBtn: HTMLButtonElement;
  readonly redoBtn: HTMLButtonElement;
  readonly importBtn: HTMLButtonElement;
  readonly exportBtn: HTMLButtonElement;
  readonly exportVoxBtn: HTMLButtonElement;
  readonly exportCvoxBtn: HTMLButtonElement;
  readonly revoxelizeBtn: HTMLButtonElement;
  readonly camOrbit: HTMLButtonElement;
  readonly camFly: HTMLButtonElement;
  readonly file: HTMLInputElement;
  readonly status: HTMLElement;
  readonly bar: BuildBar;
  readonly hud: Hud;
}

export function bindUi(): Ui {
  return {
    canvas: must(document, "#view", HTMLCanvasElement),
    toggleEdit: must(document, "#toggle-edit", HTMLButtonElement),
    toggleScene: must(document, "#toggle-scene", HTMLButtonElement),
    toggleIo: must(document, "#toggle-io", HTMLButtonElement),
    toggleStats: must(document, "#toggle-stats", HTMLButtonElement),
    panelEdit: must(document, "#panel-edit", HTMLElement),
    themeToggle: must(document, "#theme-toggle", HTMLButtonElement),
    panelScene: must(document, "#panel-scene", HTMLElement),
    panelIo: must(document, "#panel-io", HTMLElement),
    panelStats: must(document, "#panel-stats", HTMLElement),
    toggleSettings: must(document, "#toggle-settings", HTMLButtonElement),
    panelSettings: must(document, "#panel-settings", HTMLElement),
    accentSwatches: ["amber", "teal", "orchid", "mint", "violet"].map((name) =>
      must(document, `#accent-${name}`, HTMLButtonElement),
    ),
    browseDemos: must(document, "#browse-demos", HTMLButtonElement),
    sceneName: must(document, "#scene-name", HTMLElement),
    gallery: must(document, "#gallery", HTMLElement),
    galleryGrid: must(document, "#gallery-grid", HTMLElement),
    galleryEmpty: must(document, "#gallery-empty", HTMLElement),
    res: must(document, "#res", HTMLSelectElement),
    truecolor: must(document, "#truecolor", HTMLInputElement),
    zup: must(document, "#zup", HTMLInputElement),
    gpuBake: must(document, "#gpu-bake", HTMLInputElement),
    toolDraw: must(document, "#tool-draw", HTMLButtonElement),
    toolErase: must(document, "#tool-erase", HTMLButtonElement),
    toolClay: must(document, "#tool-clay", HTMLButtonElement),
    toolSmooth: must(document, "#tool-smooth", HTMLButtonElement),
    toolFlatten: must(document, "#tool-flatten", HTMLButtonElement),
    toolInflate: must(document, "#tool-inflate", HTMLButtonElement),
    toolPaint: must(document, "#tool-paint", HTMLButtonElement),
    brushRadius: must(document, "#brush-radius", HTMLInputElement),
    radiusOut: must(document, "#radius-out", HTMLOutputElement),
    strength: must(document, "#strength", HTMLInputElement),
    strengthOut: must(document, "#strength-out", HTMLOutputElement),
    falloffSmooth: must(document, "#falloff-smooth", HTMLButtonElement),
    falloffLinear: must(document, "#falloff-linear", HTMLButtonElement),
    falloffSphere: must(document, "#falloff-sphere", HTMLButtonElement),
    falloffSharp: must(document, "#falloff-sharp", HTMLButtonElement),
    brushColor: must(document, "#brush-color", HTMLInputElement),
    undoBtn: must(document, "#undo", HTMLButtonElement),
    redoBtn: must(document, "#redo", HTMLButtonElement),
    importBtn: must(document, "#import", HTMLButtonElement),
    exportBtn: must(document, "#export", HTMLButtonElement),
    exportVoxBtn: must(document, "#export-vox", HTMLButtonElement),
    exportCvoxBtn: must(document, "#export-cvox", HTMLButtonElement),
    revoxelizeBtn: must(document, "#revoxelize", HTMLButtonElement),
    camOrbit: must(document, "#cam-orbit", HTMLButtonElement),
    camFly: must(document, "#cam-fly", HTMLButtonElement),
    file: must(document, "#file", HTMLInputElement),
    status: must(document, "#status", HTMLElement),
    bar: new BuildBar(must(document, "#build-bar", HTMLElement)),
    hud: new Hud({
      fps: must(document, "#stat-fps", HTMLElement),
      frame: must(document, "#stat-frame", HTMLElement),
      frames: must(document, "#stat-frames", HTMLElement),
      nodes: must(document, "#stat-nodes", HTMLElement),
      leaves: must(document, "#stat-leaves", HTMLElement),
      voxels: must(document, "#stat-voxels", HTMLElement),
      res: must(document, "#stat-res", HTMLElement),
      heap: must(document, "#stat-heap", HTMLElement),
    }),
  };
}

/** Human wording for the kernel's phases (the kernel names what is happening;
 * the shell formats it). One table across every job; `satisfies` keeps it
 * complete — a new phase key fails the build here rather than falling through
 * silently. Wording is generic per key so a phase shared by several jobs (e.g.
 * `parse`, `assemble`) reads sensibly wherever it appears. */
export const PHASE_LABEL = {
  parse: "parsing",
  voxelize: "voxelizing on the GPU",
  compact: "compacting materials",
  cutout: "applying alpha cutout",
  assemble: "assembling structure",
  colorBake: "baking voxel colors",
  generate: "generating",
  gather: "gathering voxels",
  write: "encoding",
  pack: "packing scene",
  install: "uploading to renderer",
} satisfies Record<ProgressPhase, string>;

/** Runs a job while the segmented build bar tracks its phase stream: shows the
 * bar over `phases`, forwards each progress event to it and the status line
 * (prefixed with `context`), and always hides it when the job settles. Returns
 * the job's result. The one place the bar's lifecycle lives, so every long job
 * drives it identically. */
async function withBar<T>(
  ui: Ui,
  phases: readonly ProgressPhase[],
  context: string,
  job: (onProgress: (progress: JobProgress) => void) => Promise<T>,
): Promise<T> {
  ui.bar.begin(phases);
  try {
    const result = await job((progress) => {
      ui.bar.update(progress);
      setStatus(ui, `${context}: ${PHASE_LABEL[progress.phase]}…`);
    });
    ui.bar.finish(); // show the completed bar briefly, then auto-hide
    return result;
  } catch (e) {
    ui.bar.end(); // straight down on failure — no fake completion
    throw e;
  }
}

/** Hands `blob` to the browser as a named download. */
export function download(blob: Blob, filename: string): void {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
  URL.revokeObjectURL(a.href);
}

function showOverlay(title: string, body: string): void {
  must(document, "#overlay-title", HTMLElement).textContent = title;
  must(document, "#overlay-body", HTMLElement).textContent = body;
  must(document, "#overlay", HTMLElement).hidden = false;
}

/** Auto-fade handle for informational statuses (errors persist). */
let statusTimer: ReturnType<typeof setTimeout> | undefined;

function setStatus(ui: Ui, text: string, isError = false): void {
  ui.status.textContent = text;
  ui.status.classList.toggle("error", isError);
  // The bottom-centre status fades with relevance: an informational message
  // clears itself after a beat (the `#status:empty` CSS carries the fade);
  // errors stay until something replaces them.
  if (statusTimer !== undefined) {
    clearTimeout(statusTimer);
    statusTimer = undefined;
  }
  if (text !== "" && !isError) {
    statusTimer = setTimeout(() => {
      if (ui.status.textContent === text) {
        ui.status.textContent = "";
      }
    }, 5000);
  }
}

/** The canvas's device-pixel size (measured only — after a worker handoff the
 * main thread must not touch the backing store). */
export function measureCanvas(canvas: HTMLCanvasElement): { width: number; height: number } {
  const dpr = Math.min(window.devicePixelRatio, MAX_DPR);
  return {
    width: Math.max(1, Math.round(canvas.clientWidth * dpr)),
    height: Math.max(1, Math.round(canvas.clientHeight * dpr)),
  };
}

/** Replaces a canvas whose control may already be transferred (a failed worker
 * start leaves it unusable) with a fresh identical element. */
export function resetCanvas(old: HTMLCanvasElement): HTMLCanvasElement {
  const fresh = document.createElement("canvas");
  fresh.id = old.id;
  const label = old.getAttribute("aria-label");
  if (label !== null) {
    fresh.setAttribute("aria-label", label);
  }
  old.replaceWith(fresh);
  return fresh;
}

/** How the current scene was produced — its *provenance*. Retained so a
 * parameter change re-derives from the right thing (not always a fixture), and
 * so the shell can gate controls by what the source actually supports:
 *
 * - `fixture` — a named procedural build; resolution re-derives it (immediate).
 * - `mesh` — an imported model, its `File` **kept** so the shell can re-sample
 *   it at a new resolution or with different voxelization options. Those
 *   changes stage (an explicit re-voxelize), since re-sampling a large mesh is
 *   a multi-second job not worth firing on every picker nudge.
 * - `voxel` — a `.vox`/`.cvox` file: discrete voxels with nothing to re-sample,
 *   so build parameters do not apply.
 */
type SceneSource =
  | { readonly kind: "fixture"; readonly name: string }
  | { readonly kind: "mesh"; readonly file: File; readonly format: MeshFormat }
  | { readonly kind: "voxel" };

/** The persisted theme override ("light" | "dark"; absent = follow the OS). */
const THEME_KEY = "voxel-web.theme";

/** Packs sRGB bytes as RGBA8, R low (the boundary's colour convention). */
function packRgb(r: number, g: number, b: number): number {
  return (r | (g << 8) | (b << 16) | (0xff << 24)) >>> 0;
}

/** Resolves a CSS custom property to packed RGBA8 by letting the browser
 * compute it (probe element) and a 2D canvas parse it — exact whatever the
 * token's colour space (oklch, color-mix). `undefined` where unavailable
 * (the test DOM), in which case callers fall back to baked approximations. */
function cssColorU32(varName: string): number | undefined {
  try {
    const probe = document.createElement("div");
    probe.style.color = `var(${varName})`;
    document.body.appendChild(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    if (resolved === "") {
      return undefined;
    }
    const canvas = document.createElement("canvas");
    canvas.width = 1;
    canvas.height = 1;
    const ctx2d = canvas.getContext("2d");
    if (!ctx2d) {
      return undefined;
    }
    ctx2d.fillStyle = resolved;
    ctx2d.fillRect(0, 0, 1, 1);
    const d = ctx2d.getImageData(0, 0, 1, 1).data;
    return packRgb(d[0] ?? 0, d[1] ?? 0, d[2] ?? 0);
  } catch {
    return undefined;
  }
}

/** Baked approximations of the `--sky-top`/`--sky-bottom` token mixes, used
 * only where CSS resolution is unavailable. */
const SKY_FALLBACK = {
  dark: [packRgb(60, 66, 74), packRgb(30, 35, 41)],
  light: [packRgb(247, 242, 230), packRgb(219, 213, 199)],
} as const;

/** The persisted per-panel visibility (JSON `{ edit, scene, io, stats }`). */
const PANELS_KEY = "voxel-web.panels";

type PanelId = "edit" | "scene" | "io" | "stats" | "settings";

/** First-run composition: the brush (the tool in hand) and the scene config
 * open; file and stats folded until wanted. */
const PANEL_DEFAULTS: Record<PanelId, boolean> = {
  edit: true,
  scene: true,
  io: false,
  stats: false,
  settings: false,
};

/** The persisted accent choice; "amber" (the default) clears the override. */
const ACCENT_KEY = "voxel-web.accent";
const ACCENTS = ["amber", "teal", "orchid", "mint", "violet"] as const;
type AccentId = (typeof ACCENTS)[number];

export function run(
  host: RenderHost,
  io: IoJobs,
  ui: Ui,
  initialScene: SceneMeta,
  demoList: readonly Demo[] = listDemos(),
): void {
  // The dock toggles: each surface opens/closes on its own button (the
  // popover model — a panel hangs beneath its toggle, whose tint mirrors
  // aria-expanded). Choices persist per panel. Status and the build bar live
  // outside every toggle, so nothing job- or error-shaped can be hidden.
  const panelDock: readonly [PanelId, HTMLButtonElement, HTMLElement][] = [
    ["edit", ui.toggleEdit, ui.panelEdit],
    ["scene", ui.toggleScene, ui.panelScene],
    ["io", ui.toggleIo, ui.panelIo],
    ["stats", ui.toggleStats, ui.panelStats],
    ["settings", ui.toggleSettings, ui.panelSettings],
  ];
  const panelOpen: Record<PanelId, boolean> = { ...PANEL_DEFAULTS };
  try {
    const stored = localStorage.getItem(PANELS_KEY);
    if (stored !== null) {
      Object.assign(panelOpen, JSON.parse(stored) as Partial<Record<PanelId, boolean>>);
    }
  } catch {
    // Storage unavailable or corrupt: the first-run defaults stand.
  }
  const applyPanels = (): void => {
    for (const [id, btn, panel] of panelDock) {
      panel.hidden = !panelOpen[id];
      btn.setAttribute("aria-expanded", String(panelOpen[id]));
    }
  };
  applyPanels();
  for (const [id, btn] of panelDock) {
    btn.addEventListener("click", () => {
      panelOpen[id] = !panelOpen[id];
      applyPanels();
      try {
        localStorage.setItem(PANELS_KEY, JSON.stringify(panelOpen));
      } catch {
        // Storage unavailable: the choice just doesn't persist.
      }
    });
  }

  // The accent choice: one attribute swaps the four accent seed tones and
  // every consumer re-derives (see the data-accent CSS blocks). Amber is the
  // default and clears the override.
  const applyAccent = (accent: AccentId): void => {
    if (accent === "amber") {
      delete document.documentElement.dataset["accent"];
    } else {
      document.documentElement.dataset["accent"] = accent;
    }
    for (const [i, name] of ACCENTS.entries()) {
      ui.accentSwatches[i]?.setAttribute("aria-pressed", String(name === accent));
    }
  };
  const isAccent = (v: string | null): v is AccentId => ACCENTS.includes(v as AccentId);
  try {
    const stored = localStorage.getItem(ACCENT_KEY);
    if (isAccent(stored)) {
      applyAccent(stored);
    }
  } catch {
    // Storage unavailable: the default accent stands.
  }
  for (const [i, name] of ACCENTS.entries()) {
    ui.accentSwatches[i]?.addEventListener("click", () => {
      applyAccent(name);
      try {
        localStorage.setItem(ACCENT_KEY, name);
      } catch {
        // Storage unavailable: the choice just doesn't persist.
      }
    });
  }

  // The theme override: absent = follow the OS; a click flips away from the
  // currently *effective* mode and persists (the token seeds re-derive the
  // whole palette — this just swaps the seed set).
  const effectiveTheme = (): "light" | "dark" =>
    (document.documentElement.dataset["theme"] as "light" | "dark" | undefined) ??
    (matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark");
  // The renderer's background follows the theme tokens: resolve the sky
  // endpoints from the live stylesheet and hand them to the kernel (which
  // dithers the ramp against banding). Re-sent on every theme change.
  const syncBackground = (): void => {
    const theme = effectiveTheme();
    const top = cssColorU32("--sky-top") ?? SKY_FALLBACK[theme][0];
    const bottom = cssColorU32("--sky-bottom") ?? SKY_FALLBACK[theme][1];
    host.setBackground(top, bottom);
  };
  try {
    const stored = localStorage.getItem(THEME_KEY);
    if (stored === "light" || stored === "dark") {
      document.documentElement.dataset["theme"] = stored;
    }
  } catch {
    // Storage unavailable: follow the OS.
  }
  syncBackground();
  try {
    matchMedia("(prefers-color-scheme: light)").addEventListener("change", syncBackground);
  } catch {
    // Older engines without MediaQueryList events: the boot sync stands.
  }
  ui.themeToggle.addEventListener("click", () => {
    const next = effectiveTheme() === "light" ? "dark" : "light";
    document.documentElement.dataset["theme"] = next;
    try {
      localStorage.setItem(THEME_KEY, next);
    } catch {
      // Storage unavailable: the choice just doesn't persist.
    }
    syncBackground();
  });

  // The current scene's provenance and derived UI state.
  let source: SceneSource = { kind: "fixture", name: DEFAULT_OPTIONS.fixture };
  let editable = true;
  // Whether the current scene is truecolor — gates the Paint tool.
  let truecolor = false;
  // A mesh build parameter changed since the current mesh scene was built:
  // the re-voxelize prompt is shown until it is applied or the source changes.
  let staged = false;
  // One IO job at a time is a UI decision (no surprise scene overwrites), not
  // an engine constraint: rendering and input keep running while the worker
  // builds.
  let jobActive = false;

  /** The single source of truth for every gated control's idle state: what the
   * current source supports, plus the brush's editability gate. A no-op while
   * a job runs — `setJobActive(false)` re-gates when the job ends. */
  const gateControls = (): void => {
    if (jobActive) {
      return;
    }
    ui.browseDemos.disabled = false;
    ui.importBtn.disabled = false;
    // Voxelization options (including resolution) apply only to a mesh source —
    // a loaded demo or an import. The procedural backdrop is fixed, and a
    // discrete .vox/.cvox file has nothing to re-sample.
    const meshOpts = source.kind === "mesh";
    for (const el of [ui.truecolor, ui.zup, ui.gpuBake]) {
      el.disabled = !meshOpts;
      el.title = meshOpts ? "" : "applies when voxelizing a model or mesh import";
    }
    ui.res.disabled = !meshOpts;
    ui.res.title = meshOpts
      ? ""
      : source.kind === "voxel"
        ? "resolution is fixed by the .vox/.cvox file"
        : "load a model to resample it at another resolution";
    // Brush tools: every tool works on every editable scene since Stage D —
    // painting a palette scene promotes it to per-voxel colour on the first
    // stroke (no prompt; the status line narrates the conversion).
    for (const [btn] of toolButtons) {
      btn.disabled = !editable;
    }
    for (const el of [ui.brushRadius, ui.strength, ui.brushColor]) {
      el.disabled = !editable;
    }
    for (const [btn] of falloffButtons) {
      btn.disabled = !editable;
    }
    // History buttons re-gate from their own state (depths + job).
    syncHistoryUi();
    // Re-voxelize is a permanent resident that arms only when a mesh source
    // has a staged parameter change — disabled is the "nothing to commit"
    // state, so the control teaches its own lifecycle.
    const canRestage = source.kind === "mesh" && staged;
    ui.revoxelizeBtn.disabled = !canRestage;
    ui.revoxelizeBtn.title = canRestage
      ? "rebuild the import with the changed settings"
      : source.kind === "mesh"
        ? "no settings changed since this build"
        : "applies when voxelizing a mesh import";
  };

  // Camera control scheme (the HUD mode buttons). The active button is the UI's
  // source of truth; a new scene load re-syncs it to the ambient-spinning
  // orbit the engine resets to.
  const cameraButtons: readonly [HTMLButtonElement, CameraMode][] = [
    [ui.camOrbit, CameraMode.Orbit],
    [ui.camFly, CameraMode.Fly],
  ];
  /** Reflects the active mode in the segmented-control styling. */
  const syncCameraModeUi = (mode: CameraMode): void => {
    for (const [btn, m] of cameraButtons) {
      btn.classList.toggle("active", m === mode);
    }
  };
  /** Selects a camera mode: tells the engine and updates the buttons. */
  const selectCameraMode = (mode: CameraMode): void => {
    host.setCameraMode(mode);
    syncCameraModeUi(mode);
  };
  for (const [btn, mode] of cameraButtons) {
    btn.addEventListener("click", () => {
      selectCameraMode(mode);
    });
  }

  // Brush tool palette (segmented, like the camera modes). The active tool plus
  // the parameter inputs are the UI's source of truth; every change re-pushes
  // the whole brush config to the kernel (the rare control-plane `set_brush`).
  const toolButtons: readonly [HTMLButtonElement, BrushTool][] = [
    [ui.toolDraw, BrushTool.Draw],
    [ui.toolErase, BrushTool.Erase],
    [ui.toolClay, BrushTool.Clay],
    [ui.toolSmooth, BrushTool.Smooth],
    [ui.toolFlatten, BrushTool.Flatten],
    [ui.toolInflate, BrushTool.Inflate],
    [ui.toolPaint, BrushTool.Paint],
  ];
  // The falloff segmented picker (four curves at a glance).
  const falloffButtons: readonly [HTMLButtonElement, Falloff][] = [
    [ui.falloffSmooth, Falloff.Smooth],
    [ui.falloffLinear, Falloff.Linear],
    [ui.falloffSphere, Falloff.Sphere],
    [ui.falloffSharp, Falloff.Sharp],
  ];
  let brushFalloff: Falloff = Falloff.Smooth;
  let brushTool: BrushTool = BrushTool.Draw;
  // Alt held: the inverted tool arm (Inflate → deflate); rides set_brush.
  let invertHeld = false;
  /** "#rrggbb" → packed sRGB RGBA8 (R low), opaque. */
  const colorToU32 = (hex: string): number => {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return (r | (g << 8) | (b << 16) | (0xff << 24)) >>> 0;
  };
  const pushBrush = (): void => {
    const radius = Number(ui.brushRadius.value) || 3;
    const strength = Number(ui.strength.value);
    // The sliders' live readouts (their <output> elements) ride every push.
    ui.radiusOut.textContent = String(radius);
    ui.strengthOut.textContent = `${Math.round(strength * 100)}%`;
    host.setBrush(
      brushTool,
      radius,
      strength,
      brushFalloff,
      colorToU32(ui.brushColor.value),
      invertHeld,
    );
  };
  const syncToolUi = (): void => {
    for (const [btn, t] of toolButtons) {
      btn.classList.toggle("active", t === brushTool);
    }
  };
  const selectTool = (t: BrushTool): void => {
    brushTool = t;
    syncToolUi();
    pushBrush();
  };
  for (const [btn, t] of toolButtons) {
    btn.addEventListener("click", () => {
      selectTool(t);
    });
  }
  for (const el of [ui.brushRadius, ui.strength, ui.brushColor]) {
    el.addEventListener("input", pushBrush);
  }
  const selectFalloff = (f: Falloff): void => {
    brushFalloff = f;
    for (const [btn, v] of falloffButtons) {
      btn.classList.toggle("active", v === f);
    }
    pushBrush();
  };
  for (const [btn, f] of falloffButtons) {
    btn.addEventListener("click", () => {
      selectFalloff(f);
    });
  }

  // Stroke history. Depths arrive with every stats sample (the same 250 ms
  // cadence as the HUD counters); the buttons disable at zero and carry the
  // depth in their label so finite history is visible, not silent.
  let undoDepth = 0;
  let redoDepth = 0;
  const syncHistoryUi = (): void => {
    ui.undoBtn.disabled = jobActive || undoDepth === 0;
    ui.redoBtn.disabled = jobActive || redoDepth === 0;
    ui.undoBtn.textContent = undoDepth > 0 ? `undo ×${undoDepth}` : "undo";
    ui.redoBtn.textContent = redoDepth > 0 ? `redo ×${redoDepth}` : "redo";
  };
  const doUndo = (): void => {
    if (editable && !jobActive) {
      host.undo();
    }
  };
  const doRedo = (): void => {
    if (editable && !jobActive) {
      host.redo();
    }
  };
  ui.undoBtn.addEventListener("click", doUndo);
  ui.redoBtn.addEventListener("click", doRedo);

  const applyScene = (scene: SceneMeta): void => {
    ui.hud.setScene(scene);
    editable = scene.editable;
    truecolor = scene.truecolor;
    // A scene install clears the kernel's undo ring; reflect that now rather
    // than waiting out the next stats tick.
    undoDepth = 0;
    redoDepth = 0;
    syncHistoryUi();
    gateControls();
  };
  applyScene(initialScene);
  pushBrush(); // seed the kernel with the initial brush config

  const setJobActive = (b: boolean): void => {
    jobActive = b;
    if (b) {
      for (const el of [
        ui.browseDemos,
        ui.res,
        ui.truecolor,
        ui.zup,
        ui.gpuBake,
        ui.importBtn,
        ui.revoxelizeBtn,
        ui.toolDraw,
        ui.toolErase,
        ui.toolClay,
        ui.toolSmooth,
        ui.toolFlatten,
        ui.toolInflate,
        ui.toolPaint,
        ui.brushRadius,
        ui.strength,
        ui.falloffSmooth,
        ui.falloffLinear,
        ui.falloffSphere,
        ui.falloffSharp,
        ui.brushColor,
        ui.undoBtn,
        ui.redoBtn,
      ]) {
        el.disabled = true;
      }
    } else {
      gateControls();
    }
  };
  const withJob = async (job: () => Promise<void>): Promise<void> => {
    if (jobActive) {
      return;
    }
    setJobActive(true);
    try {
      await job();
    } finally {
      setJobActive(false);
    }
  };

  /** The current mesh-voxelization options read from the pickers. */
  const meshSettings = (): VoxelizeOptions => ({
    res: Number(ui.res.value),
    truecolor: ui.truecolor.checked,
    rotX: ui.zup.checked ? -90 : 0,
    gpuBake: ui.gpuBake.checked,
  });

  /** Installs a freshly built scene blob, adopting `newSource` as the current
   * provenance and clearing any staged change. Keeps the camera when the build
   * is a *re-derivation* of the current scene (a resolution change, a mesh
   * re-voxelize) rather than a new load — settings changes shouldn't snap the
   * view back to the default orbit. Identity: same fixture name, or the same
   * retained mesh `File`. */
  const install = async (
    blob: Uint8Array,
    label: string,
    onProgress: (progress: JobProgress) => void,
    newSource: SceneSource,
  ): Promise<void> => {
    // A re-derivation of the current scene (a mesh re-voxelize at a new
    // resolution/options — same retained File) keeps the camera; a new load
    // frames afresh.
    const preserveCamera =
      source.kind === "mesh" && newSource.kind === "mesh" && source.file === newSource.file;
    // The render-worker install is the shell's await, so the shell reports it
    // (indeterminate — no counts cross that worker's protocol).
    onProgress({ phase: "install", done: 0, total: 0 });
    const meta = await host.installScene(blob, label, preserveCamera);
    // A new load reset the engine's camera to the ambient-spinning orbit;
    // re-sync the buttons. A re-derivation kept the mode, so leave them.
    if (!preserveCamera) {
      syncCameraModeUi(CameraMode.Orbit);
    }
    source = newSource;
    staged = false;
    ui.sceneName.textContent = label;
    applyScene(meta);
  };

  attachInput(ui.canvas, {
    key: (action, down) => {
      host.key(action, down);
    },
    pointerDelta: (dx, dy) => {
      host.pointerDelta(dx, dy);
    },
    lookEnd: () => {
      host.lookEnd();
    },
    pan: (dx, dy) => {
      host.pan(dx, dy);
    },
    resetPivot: () => {
      host.resetPivot();
    },
    wheel: (notches) => {
      host.wheel(notches);
    },
    brush: (x, y, pressure) => {
      if (editable) {
        // The first Paint stroke on a palette scene promotes it — narrate
        // the (install-class) conversion before the hitch lands.
        if (brushTool === BrushTool.Paint && !truecolor) {
          setStatus(ui, "converting scene to paintable color…");
        }
        host.brush(x, y, pressure);
      }
    },
    brushEnd: () => {
      host.brushEnd();
    },
    hover: (x, y) => {
      host.hover(x, y);
    },
    undo: doUndo,
    redo: doRedo,
    tool: (t) => {
      if (editable && !jobActive) {
        selectTool(t);
      }
    },
    radiusDelta: (delta) => {
      if (editable && !jobActive) {
        const next = Math.min(12, Math.max(1, (Number(ui.brushRadius.value) || 3) + delta));
        ui.brushRadius.value = String(next);
        pushBrush();
      }
    },
    invert: (active) => {
      if (invertHeld !== active) {
        invertHeld = active;
        pushBrush();
      }
    },
  });

  /** Re-samples the retained mesh `File` with the current voxelization options
   * — the staged-apply action for a mesh source. */
  const revoxelize = async (): Promise<void> => {
    if (source.kind !== "mesh") {
      return;
    }
    const { file, format } = source;
    try {
      await withBar(ui, MESH_PHASES, file.name, async (onProgress) => {
        const bytes = new Uint8Array(await file.arrayBuffer());
        const blob = await io.voxelizeMesh(bytes, format, meshSettings(), onProgress);
        await install(blob, file.name, onProgress, { kind: "mesh", file, format });
      });
      setStatus(ui, "");
    } catch (e) {
      setStatus(ui, message(e), true);
    }
  };

  /** Marks the current mesh scene stale so its build parameters can be applied
   * on demand (via [re-voxelize]) rather than on every keystroke. */
  const markStaged = (): void => {
    if (source.kind === "mesh") {
      staged = true;
      gateControls();
    }
  };

  // The curated portfolio demos (docs/design/demo-assets.md §8), shown in the
  // gallery launcher below. Each demo rides the ordinary mesh-import path; an
  // in-session cache means re-picking one never re-bakes (design §6 stage 1).
  const demoCache = new Map<string, { readonly file: File; readonly scene: Uint8Array }>();

  /** Loads a packed demo: fetch → inflate → the real voxelize path → install.
   * The manifest's per-model choices seed the pickers first, so a later
   * resolution/option change re-voxelizes the demo like any mesh import. */
  const loadDemo = async (demo: Demo): Promise<void> => {
    ui.res.value = String(demo.res);
    ui.truecolor.checked = demo.truecolor;
    ui.zup.checked = demo.zUp;
    ui.gpuBake.checked = demo.gpuBake;
    try {
      await withBar(ui, MESH_PHASES, demo.title, async (onProgress) => {
        const hit = demoCache.get(demo.id);
        if (hit) {
          // Cached bake: skip fetch + voxelize entirely (design §6). Re-adopt
          // the retained File so a later re-voxelize still has the mesh bytes.
          await install(hit.scene.slice(), demo.title, onProgress, {
            kind: "mesh",
            file: hit.file,
            format: MeshFormat.Glb,
          });
          return;
        }
        const glb = await fetchDemoMesh(demo.url);
        // The File is built before voxelizing: voxelizeMesh transfers (neuters)
        // its input buffer, and a re-voxelize needs those mesh bytes back.
        const file = new File([glb], `${demo.id}.glb`, { type: "model/gltf-binary" });
        const scene = await io.voxelizeMesh(
          glb,
          MeshFormat.Glb,
          { res: demo.res, truecolor: demo.truecolor, rotX: demo.zUp ? -90 : 0, gpuBake: demo.gpuBake },
          onProgress,
        );
        demoCache.set(demo.id, { file, scene: scene.slice() });
        await install(scene, demo.title, onProgress, {
          kind: "mesh",
          file,
          format: MeshFormat.Glb,
        });
      });
      setStatus(ui, "");
    } catch (e) {
      setStatus(ui, message(e), true);
    }
  };

  // ── Gallery launcher ──────────────────────────────────────────────────────
  const openGallery = (): void => {
    ui.gallery.hidden = false;
  };
  const closeGallery = (): void => {
    ui.gallery.hidden = true;
  };

  // One card per demo (or the empty state). Each card closes over its Demo;
  // picking one dismisses the gallery and loads through the real voxelize path,
  // so the progress bar plays over the backdrop.
  if (demoList.length === 0) {
    ui.galleryEmpty.hidden = false;
  } else {
    for (const demo of demoList) {
      const card = document.createElement("button");
      card.type = "button";
      card.className = "demo-card";
      card.setAttribute("aria-label", `load ${demo.title}`);

      const thumb = document.createElement("span");
      thumb.className = "demo-thumb";
      if (demo.thumbnail !== null) {
        const img = document.createElement("img");
        img.src = demo.thumbnail;
        img.alt = "";
        img.loading = "lazy";
        thumb.appendChild(img);
      } else {
        // No hand-authored thumbnail yet → a monogram placeholder.
        const mono = document.createElement("span");
        mono.className = "demo-monogram";
        mono.textContent = demo.title.slice(0, 1);
        thumb.appendChild(mono);
      }

      const meta = document.createElement("span");
      meta.className = "demo-meta";
      const title = document.createElement("span");
      title.className = "demo-title";
      title.textContent = demo.title;
      meta.appendChild(title);
      if (demo.attribution !== null) {
        const attr = document.createElement("span");
        attr.className = "demo-attr";
        attr.textContent = demo.attribution;
        meta.appendChild(attr);
      }

      card.append(thumb, meta);
      card.addEventListener("click", () => {
        closeGallery();
        void withJob(() => loadDemo(demo));
      });
      ui.galleryGrid.appendChild(card);
    }
  }

  ui.browseDemos.addEventListener("click", openGallery);
  // Dismiss on the scrim or close button (both carry data-gallery-dismiss), or
  // on Escape while the gallery is open.
  ui.gallery.addEventListener("click", (e) => {
    if (e.target instanceof Element && e.target.closest("[data-gallery-dismiss]")) {
      closeGallery();
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !ui.gallery.hidden) {
      closeGallery();
    }
  });
  // The launcher greets every visit when there is something to show; a checkout
  // with nothing packed just keeps the backdrop and reaches the gallery (empty
  // state) via the button.
  if (demoList.length > 0) {
    openGallery();
  }

  // Resolution and the voxelization options apply to a loaded model / mesh
  // import (disabled otherwise); changing one stages a re-voxelize.
  ui.res.addEventListener("change", markStaged);
  // Voxelization options only exist for a mesh source (disabled otherwise);
  // changing one stages a re-voxelize.
  for (const el of [ui.truecolor, ui.zup, ui.gpuBake]) {
    el.addEventListener("change", markStaged);
  }
  ui.revoxelizeBtn.addEventListener("click", () => {
    void withJob(revoxelize);
  });

  const importFile = async (file: File): Promise<void> => {
    const target = importKindFor(file.name);
    if (target === undefined) {
      setStatus(ui, `unsupported file type: ${file.name} (glb/gltf/obj/stl/vox/cvox)`, true);
      return;
    }
    try {
      const phases = target.kind === "mesh" ? MESH_PHASES : DECODE_PHASES;
      await withBar(ui, phases, file.name, async (onProgress) => {
        // The read is inside the bar too: a multi-hundred-MB file takes real
        // time before the kernel's first phase can fire.
        const bytes = new Uint8Array(await file.arrayBuffer());
        if (target.kind === "vox" || target.kind === "cvox") {
          // Voxel-native: no voxelization pass; the grid auto-sizes to the model.
          const blob =
            target.kind === "vox"
              ? await io.decodeVox(bytes, onProgress)
              : await io.decodeCvox(bytes, onProgress);
          await install(blob, file.name, onProgress, { kind: "voxel" });
        } else {
          // Retain the File (not the bytes) so a later resolution/option change
          // can re-sample the same model without re-picking it.
          const blob = await io.voxelizeMesh(bytes, target.format, meshSettings(), onProgress);
          await install(blob, file.name, onProgress, {
            kind: "mesh",
            file,
            format: target.format,
          });
        }
      });
      setStatus(ui, "");
    } catch (e) {
      setStatus(ui, message(e), true);
    }
  };

  ui.importBtn.addEventListener("click", () => {
    ui.file.click();
  });
  ui.file.addEventListener("change", () => {
    const file = ui.file.files?.[0];
    ui.file.value = ""; // re-selecting the same file must re-fire "change"
    if (file) {
      void withJob(() => importFile(file));
    }
  });

  // Drag-and-drop anywhere on the page voxelizes the dropped model.
  window.addEventListener("dragover", (e) => {
    e.preventDefault();
  });
  window.addEventListener("drop", (e) => {
    e.preventDefault();
    const file = e.dataTransfer?.files[0];
    if (file) {
      void withJob(() => importFile(file));
    }
  });

  // PNG export captures the canvas's current image on the main thread — valid
  // for both hosts: a transferred canvas still displays the worker's frames.
  ui.exportBtn.addEventListener("click", () => {
    ui.canvas.toBlob((blob) => {
      if (!blob) {
        setStatus(ui, "png capture failed", true);
        return;
      }
      download(blob, "voxel-web.png");
    }, "image/png");
  });

  // Voxel-native structure downloads: the render host snapshots the scene
  // (worker-side, transferred back), the IO worker does the encode — the main
  // thread only routes blobs and saves the file.
  const exportScene = (kind: "vox" | "cvox", filename: string): Promise<void> =>
    withJob(async () => {
      try {
        // The snapshot is a fast worker→main copy; the encode (gather + write)
        // is the metered part, so the bar covers the encode job.
        const scene = await host.snapshotScene();
        const encoded = await withBar(ui, ENCODE_PHASES, filename, (onProgress) =>
          kind === "vox" ? io.encodeVox(scene, onProgress) : io.encodeCvox(scene, onProgress),
        );
        const bytes = new Uint8Array(encoded);
        download(new Blob([bytes], { type: "application/octet-stream" }), filename);
        setStatus(ui, "");
      } catch (e) {
        setStatus(ui, message(e), true);
      }
    });
  ui.exportVoxBtn.addEventListener("click", () => {
    void exportScene("vox", "voxel-web.vox");
  });
  ui.exportCvoxBtn.addEventListener("click", () => {
    void exportScene("cvox", "voxel-web.cvox");
  });

  // Interactive resizes fire the observer at high frequency, and each resize
  // reallocates the render-output texture — coalesce to one per frame.
  let resizeQueued = false;
  const observer = new ResizeObserver(() => {
    if (resizeQueued) {
      return;
    }
    resizeQueued = true;
    requestAnimationFrame(() => {
      resizeQueued = false;
      const { width, height } = measureCanvas(ui.canvas);
      host.resize(width, height);
    });
  });
  observer.observe(ui.canvas);

  host.onStats((stats) => {
    ui.hud.setStats(stats);
    undoDepth = stats.undoDepth;
    redoDepth = stats.redoDepth;
    syncHistoryUi();
    // The kernel's truecolor flag flipping mid-scene is the promotion
    // completing (first Paint stroke on a palette scene).
    if (stats.truecolor && !truecolor) {
      truecolor = true;
      setStatus(ui, "scene promoted to paintable color");
    }
  });
  io.onHeap((bytes) => {
    ui.hud.setIoHeap(bytes);
  });
}

/** The boot-time constructors, injectable so tests can exercise the fallback
 * ladder without real workers or WebGPU. Production uses the defaults. */
export interface BootDeps {
  createWorkerHost(): RenderHost;
  createLocalHost(): RenderHost;
  createIo(): IoJobs;
}

const DEFAULT_BOOT: BootDeps = {
  createWorkerHost: () => new WorkerRenderHost(),
  createLocalHost: () => new LocalRenderHost(),
  createIo: () => new IoClient(),
};

export async function main(deps: BootDeps = DEFAULT_BOOT): Promise<void> {
  const ui = bindUi();
  if (!("gpu" in navigator)) {
    showOverlay(
      "WebGPU unavailable",
      "This renderer is compute-shader based and needs WebGPU. " +
        "Any current Chrome, Edge, Firefox, or Safari release has it.",
    );
    return;
  }
  const io = deps.createIo(); // IO worker boots in parallel with the renderer
  try {
    setStatus(ui, `starting renderer (${DEFAULT_OPTIONS.fixture} ${DEFAULT_OPTIONS.res}³)…`);
    let scene;
    let host: RenderHost;
    if ("transferControlToOffscreen" in ui.canvas) {
      const workerHost = deps.createWorkerHost();
      try {
        scene = await workerHost.start(ui.canvas, DEFAULT_OPTIONS);
        host = workerHost;
      } catch (e) {
        // The transferred canvas is spent; fall back on a fresh element with
        // the phase-1 topology (render on main, builds still off-thread). The
        // failed worker — and its wasm instance — must not idle for the
        // session.
        console.warn(`worker rendering unavailable (${message(e)}); rendering on main`);
        workerHost.dispose();
        ui.canvas = resetCanvas(ui.canvas);
        host = deps.createLocalHost();
        scene = await host.start(ui.canvas, DEFAULT_OPTIONS);
      }
    } else {
      host = deps.createLocalHost();
      scene = await host.start(ui.canvas, DEFAULT_OPTIONS);
    }
    setStatus(ui, "");
    run(host, io, ui, scene);
  } catch (e) {
    showOverlay("failed to start", message(e));
  }
}
