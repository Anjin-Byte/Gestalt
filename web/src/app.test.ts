// @vitest-environment happy-dom
// Shell orchestration tests, mounted on the real index.html markup (imported
// ?raw) so the DOM contract the shell binds against cannot drift from what
// these tests prove. Hosts and IO are recording fakes behind the RenderHost /
// IoJobs seams; the oracles are exact statuses, exact job arguments, and
// exact gating states.
import { BrushTool, CameraMode, Falloff, KeyAction, MeshFormat } from "voxel-web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import indexHtml from "../index.html?raw";
import { bindUi, main, measureCanvas, must, resetCanvas, run, type BootDeps, type Ui } from "./app";
import { type Demo } from "./demos";
import { type IoJobs } from "./io";
import { type JobProgress, type VoxelizeOptions } from "./io-protocol";
import { type EngineOptions, type RenderHost } from "./render";
import { type HudStats, type SceneMeta } from "./render-protocol";

function mount(): void {
  const body = /<body>([\s\S]*)<\/body>/.exec(indexHtml)?.[1];
  if (body === undefined) {
    throw new Error("index.html has no body to mount");
  }
  // The entry <script> is not part of the DOM contract under test (and
  // happy-dom would try to load it).
  document.body.innerHTML = body.replace(/<script[\s\S]*?<\/script>/g, "");
}

function makeScene(overrides: Partial<SceneMeta> = {}): SceneMeta {
  return {
    label: "wire-lattice",
    nodes: 1,
    leaves: 2,
    voxels: 3,
    res: 128,
    editable: true,
    truecolor: false,
    ...overrides,
  };
}

class FakeHost implements RenderHost {
  readonly mode = "worker" as const;
  readonly startCalls: { canvas: HTMLCanvasElement; opts: EngineOptions }[] = [];
  startResult: () => Promise<SceneMeta> = () => Promise.resolve(makeScene());
  readonly installs: { blob: Uint8Array; label: string }[] = [];
  // The preserveCamera flag per install, recorded separately so `installs`
  // assertions stay exact.
  readonly installPreserve: boolean[] = [];
  installResult: (label: string) => SceneMeta = (label) => makeScene({ label });
  snapshots = 0;
  snapshotResult: () => Promise<Uint8Array> = () => Promise.resolve(new Uint8Array([1]));
  readonly keys: [KeyAction, boolean][] = [];
  readonly deltas: [number, number][] = [];
  readonly wheels: number[] = [];
  readonly setBrushes: [BrushTool, number, number, Falloff, number, boolean][] = [];
  readonly brushes: [number, number, number][] = [];
  brushEnds = 0;
  readonly hovers: [number, number][] = [];
  readonly resizes: [number, number][] = [];
  statsCb: ((stats: HudStats) => void) | undefined;

  start(canvas: HTMLCanvasElement, opts: EngineOptions): Promise<SceneMeta> {
    this.startCalls.push({ canvas, opts });
    return this.startResult();
  }
  installScene(blob: Uint8Array, label: string, preserveCamera: boolean): Promise<SceneMeta> {
    this.installs.push({ blob, label });
    this.installPreserve.push(preserveCamera);
    return Promise.resolve(this.installResult(label));
  }
  snapshotScene(): Promise<Uint8Array> {
    this.snapshots += 1;
    return this.snapshotResult();
  }
  resize(width: number, height: number): void {
    this.resizes.push([width, height]);
  }
  key(action: KeyAction, down: boolean): void {
    this.keys.push([action, down]);
  }
  pointerDelta(dx: number, dy: number): void {
    this.deltas.push([dx, dy]);
  }
  lookEnds = 0;
  lookEnd(): void {
    this.lookEnds += 1;
  }
  readonly pans: [number, number][] = [];
  pan(dx: number, dy: number): void {
    this.pans.push([dx, dy]);
  }
  pivotResets = 0;
  resetPivot(): void {
    this.pivotResets += 1;
  }
  wheel(notches: number): void {
    this.wheels.push(notches);
  }
  readonly cameraModes: CameraMode[] = [];
  readonly gtaoCalls: boolean[] = [];
  readonly gtaoQualityCalls: number[] = [];
  readonly shadowQualityCalls: number[] = [];
  setGtao(on: boolean): void {
    this.gtaoCalls.push(on);
  }
  setGtaoQuality(preset: number): void {
    this.gtaoQualityCalls.push(preset);
  }
  setShadowQuality(quality: number): void {
    this.shadowQualityCalls.push(quality);
  }
  setCameraMode(mode: CameraMode): void {
    this.cameraModes.push(mode);
  }
  setBrush(
    tool: BrushTool,
    radius: number,
    strength: number,
    falloff: Falloff,
    color: number,
    invert: boolean,
  ): void {
    this.setBrushes.push([tool, radius, strength, falloff, color, invert]);
  }
  brush(x: number, y: number, pressure: number): void {
    this.brushes.push([x, y, pressure]);
  }
  brushEnd(): void {
    this.brushEnds += 1;
  }
  hover(x: number, y: number): void {
    this.hovers.push([x, y]);
  }
  readonly backgrounds: [number, number][] = [];
  setBackground(top: number, bottom: number): void {
    this.backgrounds.push([top, bottom]);
  }
  undos = 0;
  undo(): void {
    this.undos += 1;
  }
  redos = 0;
  redo(): void {
    this.redos += 1;
  }
  onStats(cb: (stats: HudStats) => void): void {
    this.statsCb = cb;
  }
  disposed = 0;
  dispose(): void {
    this.disposed += 1;
  }
}

class FakeIo implements IoJobs {
  readonly buildFixtureCalls: [string, number][] = [];
  buildFixtureResult: () => Promise<Uint8Array> = () => Promise.resolve(new Uint8Array([1]));
  readonly voxelizeCalls: {
    bytes: Uint8Array;
    format: MeshFormat;
    opts: VoxelizeOptions;
  }[] = [];
  voxelizeResult: () => Promise<Uint8Array> = () => Promise.resolve(new Uint8Array([2]));
  readonly decodeVoxCalls: Uint8Array[] = [];
  readonly decodeCvoxCalls: Uint8Array[] = [];
  readonly encodeVoxCalls: Uint8Array[] = [];
  readonly encodeCvoxCalls: Uint8Array[] = [];

  // The most recent progress callback handed to any job — the tests drive it
  // to exercise the bar/status wiring.
  lastOnProgress: ((progress: JobProgress) => void) | undefined;

  voxelizeMesh(
    bytes: Uint8Array,
    format: MeshFormat,
    opts: VoxelizeOptions,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array> {
    this.voxelizeCalls.push({ bytes, format, opts });
    this.lastOnProgress = onProgress;
    return this.voxelizeResult();
  }
  buildFixture(
    fixture: string,
    res: number,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array> {
    this.buildFixtureCalls.push([fixture, res]);
    this.lastOnProgress = onProgress;
    return this.buildFixtureResult();
  }
  decodeVox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    this.decodeVoxCalls.push(bytes);
    this.lastOnProgress = onProgress;
    return Promise.resolve(new Uint8Array([3]));
  }
  decodeCvox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    this.decodeCvoxCalls.push(bytes);
    this.lastOnProgress = onProgress;
    return Promise.resolve(new Uint8Array([4]));
  }
  encodeVox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    this.encodeVoxCalls.push(scene);
    this.lastOnProgress = onProgress;
    return Promise.resolve(new Uint8Array([5]));
  }
  encodeCvox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    this.encodeCvoxCalls.push(scene);
    this.lastOnProgress = onProgress;
    return Promise.resolve(new Uint8Array([6]));
  }
  heapCb: ((bytes: number) => void) | undefined;
  onHeap(cb: (bytes: number) => void): void {
    this.heapCb = cb;
  }
}

interface Deferred<T> {
  readonly promise: Promise<T>;
  readonly resolve: (value: T) => void;
  readonly reject: (error: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function flush(): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

let resizeCallback: (() => void) | undefined;
let rafQueue: FrameRequestCallback[] = [];

/** Mounts the page and wires a run() with fresh fakes. Demos are injected (not
 * read from the ambient packed set) so gallery behaviour is deterministic —
 * empty by default; gallery tests pass a known list. */
function setup(
  initial: Partial<SceneMeta> = {},
  demoList: readonly Demo[] = [],
): { ui: Ui; host: FakeHost; io: FakeIo } {
  const ui = bindUi();
  const host = new FakeHost();
  const io = new FakeIo();
  run(host, io, ui, makeScene(initial), demoList);
  return { ui, host, io };
}

function makeDemo(overrides: Partial<Demo> = {}): Demo {
  return {
    id: "tokyo",
    title: "Tokyo",
    url: "/assets/demos/tokyo.glb.gz",
    thumbnail: null,
    res: 512,
    zUp: false,
    truecolor: true,
    gpuBake: true,
    alphaMode: "opaque",
    attribution: null,
    ...overrides,
  };
}

function status(): { text: string; isError: boolean } {
  const el = must(document, "#status", HTMLElement);
  return { text: el.textContent, isError: el.classList.contains("error") };
}

function dropFile(name: string, bytes: Uint8Array): void {
  const e = new Event("drop", { cancelable: true });
  // Fresh-wrapped: BlobPart requires a Uint8Array over a plain ArrayBuffer.
  const file = new File([new Uint8Array(bytes)], name);
  Object.defineProperty(e, "dataTransfer", { value: { files: [file] } });
  window.dispatchEvent(e);
}

beforeEach(() => {
  mount();
  localStorage.clear(); // the menu-panel choice persists; tests start fresh
  resizeCallback = undefined;
  rafQueue = [];
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback): number => {
    rafQueue.push(cb);
    return rafQueue.length;
  });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(cb: () => void) {
        resizeCallback = cb;
      }
      observe(): void {
        /* recorded via the constructor callback */
      }
      unobserve(): void {
        /* not needed */
      }
      disconnect(): void {
        /* not needed */
      }
    },
  );
  URL.createObjectURL = vi.fn(() => "blob:test");
  URL.revokeObjectURL = vi.fn();
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("bindUi", () => {
  it("binds every element the shipped markup provides", () => {
    const ui = bindUi();
    expect(ui.canvas.id).toBe("view");
    expect(ui.gallery.hidden).toBe(true); // launcher starts closed
    expect(ui.browseDemos.textContent).toContain("browse");
    expect(ui.res.value).toBe("128");
    expect(ui.truecolor.checked).toBe(true);
    expect(ui.gpuBake.checked).toBe(true);
    expect(ui.zup.checked).toBe(false);
  });

  it("fails loudly when the markup contract is broken", () => {
    must(document, "#res", HTMLSelectElement).remove();
    expect(() => bindUi()).toThrow("missing/mistyped element #res");
  });
});

describe("scene application and edit gating", () => {
  it("enables every tool on every editable scene (paint promotes palette scenes)", () => {
    const { ui } = setup({ label: "wire-lattice", res: 128, editable: true, truecolor: false });
    expect(must(document, "#stat-res", HTMLElement).textContent).toBe("128³");
    expect(ui.toolDraw.disabled).toBe(false);
    expect(ui.toolErase.disabled).toBe(false);
    expect(ui.brushRadius.disabled).toBe(false);
    // Stage D: paint is never gated — the first stroke promotes the scene.
    expect(ui.toolPaint.disabled).toBe(false);
  });

  it("paint stays selected across a palette-scene install (promotion handles it)", async () => {
    const { ui, host } = setup({ editable: true, truecolor: true });
    ui.toolPaint.click();
    expect(ui.toolPaint.classList.contains("active")).toBe(true);
    host.installResult = (label) => makeScene({ label, truecolor: false });
    dropFile("model.glb", new Uint8Array([1, 2, 3]));
    await flush();
    expect(ui.toolPaint.disabled).toBe(false);
    expect(ui.toolPaint.classList.contains("active")).toBe(true);
  });

  it("narrates the promotion: converting on the first paint stroke, promoted on the stats flip", () => {
    const { ui, host } = setup({ editable: true, truecolor: false });
    ui.canvas.setPointerCapture = () => undefined;
    ui.canvas.hasPointerCapture = () => false;
    ui.toolPaint.click();
    const down = new PointerEvent("pointerdown", { button: 2, pointerId: 1 });
    Object.defineProperty(down, "offsetX", { value: 10 });
    Object.defineProperty(down, "offsetY", { value: 10 });
    ui.canvas.dispatchEvent(down);
    expect(host.brushes).toHaveLength(1);
    expect(must(document, "#status", HTMLElement).textContent).toContain("converting");
    host.statsCb?.({
      fps: 60,
      frameAvg: 16,
      frameMin: 16,
      frameMax: 16,
      frames: 1,
      nodes: 1,
      leaves: 2,
      voxels: 3,
      undoDepth: 0,
      redoDepth: 0,
      truecolor: true,
      heapBytes: 0,
    });
    expect(must(document, "#status", HTMLElement).textContent).toContain("promoted");
  });

  it("pushes the brush config to the host on tool and parameter changes", () => {
    const { ui, host } = setup({ editable: true, truecolor: true });
    expect(host.setBrushes.length).toBeGreaterThanOrEqual(1); // seeded at startup
    host.setBrushes.length = 0;
    ui.toolPaint.click();
    expect(host.setBrushes.at(-1)?.[0]).toBe(BrushTool.Paint);
    ui.brushRadius.value = "7";
    ui.brushRadius.dispatchEvent(new Event("input"));
    expect(host.setBrushes.at(-1)?.[1]).toBe(7);
    // The slider's <output> readout tracks every push.
    expect(ui.radiusOut.textContent).toBe("7");
    ui.strength.value = "0.4";
    ui.strength.dispatchEvent(new Event("input"));
    expect(ui.strengthOut.textContent).toBe("40%");
    // Falloff is a segmented picker: click selects, highlights, and pushes.
    ui.falloffSharp.click();
    expect(host.setBrushes.at(-1)?.[3]).toBe(Falloff.Sharp);
    expect(ui.falloffSharp.classList.contains("active")).toBe(true);
    expect(ui.falloffSmooth.classList.contains("active")).toBe(false);
  });

  it("the sculpt set selects and pushes its tools on any editable scene", () => {
    const { ui, host } = setup({ editable: true, truecolor: false });
    const sculpt: [HTMLButtonElement, BrushTool][] = [
      [ui.toolClay, BrushTool.Clay],
      [ui.toolSmooth, BrushTool.Smooth],
      [ui.toolFlatten, BrushTool.Flatten],
      [ui.toolInflate, BrushTool.Inflate],
    ];
    for (const [btn, tool] of sculpt) {
      expect(btn.disabled).toBe(false); // no truecolor gate on sculpt tools
      btn.click();
      expect(host.setBrushes.at(-1)?.[0]).toBe(tool);
      expect(btn.classList.contains("active")).toBe(true);
    }
  });
});

describe("lighting effects", () => {
  it("boots with AO on at medium and shadows off, and forwards changes to the host", () => {
    const { ui, host } = setup();
    // Markup defaults: the web ships AO on at Medium, shadows LOW (half-res).
    expect(ui.gtaoOn.checked).toBe(true);
    expect(ui.gtaoQuality.value).toBe("1");
    expect(ui.shadows.value).toBe("1");

    ui.gtaoOn.checked = false;
    ui.gtaoOn.dispatchEvent(new Event("change"));
    expect(host.gtaoCalls).toEqual([false]);

    ui.gtaoQuality.value = "3";
    ui.gtaoQuality.dispatchEvent(new Event("change"));
    expect(host.gtaoQualityCalls).toEqual([3]);

    ui.shadows.value = "2";
    ui.shadows.dispatchEvent(new Event("change"));
    expect(host.shadowQualityCalls).toEqual([2]);
  });
});

describe("demo gallery", () => {
  const gallery = (): HTMLElement => must(document, "#gallery", HTMLElement);
  const cards = (): HTMLButtonElement[] =>
    [...must(document, "#gallery-grid", HTMLElement).querySelectorAll("button.demo-card")].map(
      (c) => c as HTMLButtonElement,
    );

  it("stays closed with no demos and shows the empty state on demand", () => {
    const { ui } = setup(); // no demos injected
    expect(gallery().hidden).toBe(true);
    expect(cards()).toHaveLength(0);
    ui.browseDemos.click();
    expect(gallery().hidden).toBe(false);
    expect(must(document, "#gallery-empty", HTMLElement).hidden).toBe(false);
  });

  it("opens at boot and renders a card per demo (title, attribution, placeholder vs thumb)", () => {
    setup({}, [
      makeDemo({ id: "tokyo", title: "Tokyo", attribution: "by X, CC-BY-4.0" }),
      makeDemo({ id: "mask", title: "Mask", thumbnail: "/assets/demos/mask.thumb.webp" }),
    ]);
    expect(gallery().hidden).toBe(false); // launches on load
    const c = cards();
    expect(c).toHaveLength(2);
    expect(c[0]?.querySelector(".demo-title")?.textContent).toBe("Tokyo");
    expect(c[0]?.querySelector(".demo-attr")?.textContent).toBe("by X, CC-BY-4.0");
    expect(c[0]?.querySelector(".demo-monogram")?.textContent).toBe("T"); // no thumb → monogram
    expect(c[0]?.querySelector("img")).toBeNull();
    expect(c[1]?.querySelector("img")?.getAttribute("src")).toBe("/assets/demos/mask.thumb.webp");
  });

  it("closes on the close button, the scrim, and Escape", () => {
    const { ui } = setup({}, [makeDemo()]);
    expect(gallery().hidden).toBe(false);
    must(document, "#gallery-close", HTMLButtonElement).click();
    expect(gallery().hidden).toBe(true);
    ui.browseDemos.click();
    must(document, ".gallery-scrim", HTMLElement).click();
    expect(gallery().hidden).toBe(true);
    ui.browseDemos.click();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(gallery().hidden).toBe(true);
  });

  it("loads a demo through the mesh path with its manifest options, then dismisses", async () => {
    // 'glTF' magic (not gzip) → fetchDemoMesh passes the bytes straight through,
    // so no DecompressionStream is needed under happy-dom.
    const glb = new Uint8Array([0x67, 0x6c, 0x54, 0x46, 2, 0, 0, 0]);
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response(glb))));
    const { ui, io } = setup({}, [
      makeDemo({ id: "tokyo", title: "Tokyo", res: 512, zUp: true, truecolor: true, gpuBake: false }),
    ]);
    cards()[0]?.click();
    expect(gallery().hidden).toBe(true); // dismisses immediately; bar plays over the backdrop
    await flush();
    expect(io.voxelizeCalls).toHaveLength(1);
    expect(io.voxelizeCalls[0]).toMatchObject({
      format: MeshFormat.Glb,
      opts: { res: 512, truecolor: true, rotX: -90, gpuBake: false },
    });
    // The pickers were seeded from the manifest options; the scene is named.
    expect(ui.res.value).toBe("512");
    expect(ui.gpuBake.checked).toBe(false);
    expect(ui.sceneName.textContent).toBe("Tokyo");
  });

  it("re-picking a cached demo never re-fetches or re-bakes", async () => {
    const glb = new Uint8Array([0x67, 0x6c, 0x54, 0x46, 2, 0, 0, 0]);
    const fetchMock = vi.fn(() => Promise.resolve(new Response(glb)));
    vi.stubGlobal("fetch", fetchMock);
    const { ui, io } = setup({}, [makeDemo({ id: "tokyo", title: "Tokyo" })]);
    cards()[0]?.click();
    await flush();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(io.voxelizeCalls).toHaveLength(1);
    // Re-open and pick again → served from the in-session cache.
    ui.browseDemos.click();
    cards()[0]?.click();
    await flush();
    expect(fetchMock).toHaveBeenCalledTimes(1); // no second fetch
    expect(io.voxelizeCalls).toHaveLength(1); // no second bake
  });
});

describe("scene provenance and staged re-voxelize", () => {
  const revoxelize = (): HTMLButtonElement => must(document, "#revoxelize", HTMLButtonElement);

  /** Imports a mesh and settles, leaving a mesh-source scene current. */
  async function withMeshLoaded(): Promise<{ ui: Ui; host: FakeHost; io: FakeIo }> {
    const ctx = setup();
    dropFile("model.glb", new Uint8Array([1, 2, 3]));
    await flush();
    return ctx;
  }

  it("gates voxelization options + resolution to a mesh source (off the backdrop / voxel file)", async () => {
    const { ui } = setup();
    // Boot: the procedural backdrop is current — options and resolution are all
    // mesh-only (the backdrop is fixed, not tunable).
    expect(ui.truecolor.disabled).toBe(true);
    expect(ui.gpuBake.disabled).toBe(true);
    expect(ui.zup.title).toBe("applies when voxelizing a model or mesh import");
    expect(ui.res.disabled).toBe(true);

    // A .cvox: resolution is fixed by the file, options still mesh-only.
    dropFile("scene.cvox", new Uint8Array([1]));
    await flush();
    expect(ui.res.disabled).toBe(true);
    expect(ui.res.title).toBe("resolution is fixed by the .vox/.cvox file");
    expect(ui.truecolor.disabled).toBe(true);
    expect(ui.sceneName.textContent).toBe("scene.cvox"); // the current scene is named
  });

  it("enables the mesh options once a mesh is current", async () => {
    const { ui } = await withMeshLoaded();
    expect(ui.truecolor.disabled).toBe(false);
    expect(ui.zup.disabled).toBe(false);
    expect(ui.gpuBake.disabled).toBe(false);
    expect(ui.res.disabled).toBe(false);
    expect(revoxelize().disabled).toBe(true); // nothing staged yet
  });

  it("stages a resolution change on a mesh instead of rebuilding immediately", async () => {
    const { ui, io } = await withMeshLoaded();
    expect(io.voxelizeCalls).toHaveLength(1); // the import
    ui.res.value = "2048";
    ui.res.dispatchEvent(new Event("change"));
    // No rebuild — just the staged prompt.
    expect(io.voxelizeCalls).toHaveLength(1);
    expect(revoxelize().disabled).toBe(false);
    expect(io.buildFixtureCalls).toHaveLength(0);
  });

  it("re-voxelizes the retained mesh with the current options on apply", async () => {
    const { ui, host, io } = await withMeshLoaded();
    ui.res.value = "2048";
    ui.truecolor.checked = false;
    ui.res.dispatchEvent(new Event("change"));
    ui.truecolor.dispatchEvent(new Event("change"));
    expect(revoxelize().disabled).toBe(false);

    revoxelize().click();
    await flush();
    // Re-sampled the SAME file (not a fixture rebuild), with the new options.
    expect(io.voxelizeCalls).toHaveLength(2);
    expect(io.buildFixtureCalls).toHaveLength(0);
    expect(io.voxelizeCalls[1]).toMatchObject({
      format: MeshFormat.Glb,
      opts: { res: 2048, truecolor: false },
    });
    expect(host.installs).toHaveLength(2);
    expect(revoxelize().disabled).toBe(true); // staged change consumed
  });

  it("a resolution change off a mesh source neither stages nor rebuilds", async () => {
    const { io } = setup(); // the procedural backdrop is current
    must(document, "#res", HTMLSelectElement).dispatchEvent(new Event("change"));
    await flush();
    expect(io.buildFixtureCalls).toHaveLength(0);
    expect(io.voxelizeCalls).toHaveLength(0);
    expect(revoxelize().disabled).toBe(true);
  });

  it("re-voxelize does nothing when the current source is not a mesh", async () => {
    const { io } = setup(); // fixture source
    // The button is disabled, and a stray click must be inert either way.
    revoxelize().click();
    await flush();
    expect(io.voxelizeCalls).toHaveLength(0);
    expect(io.buildFixtureCalls).toHaveLength(0);
  });

  it("preserves the camera on a re-derivation, resets it on a new load", async () => {
    const { ui, host } = await withMeshLoaded();
    expect(host.installPreserve).toEqual([false]); // the initial import is a new load

    // Re-voxelizing the same mesh is a re-derivation → keep the view.
    ui.res.value = "512";
    ui.res.dispatchEvent(new Event("change"));
    revoxelize().click();
    await flush();
    expect(host.installPreserve).toEqual([false, true]);

    // Loading a different model is a new load → reset to the framing orbit.
    dropFile("other.glb", new Uint8Array([9, 9, 9]));
    await flush();
    expect(host.installPreserve).toEqual([false, true, false]);
  });
});

describe("imports", () => {
  it("rejects unsupported file types without touching the workers", async () => {
    const { io } = setup();
    dropFile("notes.txt", new Uint8Array([1]));
    await flush();
    expect(status()).toEqual({
      text: "unsupported file type: notes.txt (glb/gltf/obj/stl/vox/cvox)",
      isError: true,
    });
    expect(io.decodeVoxCalls).toHaveLength(0);
    expect(io.voxelizeCalls).toHaveLength(0);
  });

  it("routes .vox drops through the decoder, no voxelization pass", async () => {
    const { host, io } = setup();
    dropFile("castle.vox", new Uint8Array([0x56, 0x4f, 0x58, 0x20]));
    await flush();
    expect(io.decodeVoxCalls).toHaveLength(1);
    expect(io.decodeVoxCalls[0]).toEqual(new Uint8Array([0x56, 0x4f, 0x58, 0x20]));
    expect(io.voxelizeCalls).toHaveLength(0);
    expect(host.installs).toEqual([{ blob: new Uint8Array([3]), label: "castle.vox" }]);
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("routes .cvox drops through its own decoder", async () => {
    const { io } = setup();
    dropFile("scene.cvox", new Uint8Array([1, 2]));
    await flush();
    expect(io.decodeCvoxCalls).toHaveLength(1);
    expect(io.decodeVoxCalls).toHaveLength(0);
  });

  it("shows the decode bar (parse/assemble) for a .vox drop", async () => {
    const { io } = setup();
    const gate = deferred<Uint8Array>();
    // Hold the decode open so the bar is observable mid-job.
    io.decodeVox = (_bytes, onProgress) => {
      io.lastOnProgress = onProgress;
      return gate.promise;
    };
    dropFile("castle.vox", new Uint8Array([1]));
    await flush();
    const bar = must(document, "#build-bar", HTMLElement);
    expect(bar.hidden).toBe(false);
    expect(bar.children).toHaveLength(4); // parse, assemble, pack, install
    io.lastOnProgress?.({ phase: "parse", done: 0, total: 0 });
    expect(status().text).toBe("castle.vox: parsing…");
    gate.resolve(new Uint8Array([3]));
    await flush();
    expect(bar.hidden).toBe(false); // completed bar lingers before auto-hiding
  });

  it("keeps the bar up through the render-worker install (shell-emitted phase)", async () => {
    const { host, io } = setup();
    // The decode resolves instantly; the *install* is the slow part here.
    const gate = deferred<SceneMeta>();
    host.installScene = (blob, label) => {
      host.installs.push({ blob, label });
      return gate.promise;
    };
    dropFile("castle.vox", new Uint8Array([1]));
    await flush();
    const bar = must(document, "#build-bar", HTMLElement);
    // Decode finished, install pending: the bar must still be visible with
    // the shell-emitted install phase active and labelled.
    expect(io.decodeVoxCalls).toHaveLength(1);
    expect(bar.hidden).toBe(false);
    expect(status().text).toBe("castle.vox: uploading to renderer…");
    const install = bar.children[3];
    expect(install instanceof HTMLElement && install.classList.contains("active")).toBe(true);
    gate.resolve(makeScene({ label: "castle.vox" }));
    await flush();
    // Completed bar lingers (auto-hides after FINISH_LINGER_MS).
    expect(bar.hidden).toBe(false);
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("voxelizes mesh drops with the pickers' exact options", async () => {
    const { ui, io } = setup();
    ui.res.value = "512";
    ui.truecolor.checked = false;
    ui.zup.checked = true;
    ui.gpuBake.checked = false;
    dropFile("model.stl", new Uint8Array([9, 9]));
    await flush();
    expect(io.voxelizeCalls).toHaveLength(1);
    expect(io.voxelizeCalls[0]).toMatchObject({
      format: MeshFormat.Stl,
      opts: { res: 512, truecolor: false, rotX: -90, gpuBake: false },
    });
    expect(io.voxelizeCalls[0]?.bytes).toEqual(new Uint8Array([9, 9]));
  });

  it("shows the build bar during a mesh import and labels each phase", async () => {
    const { io } = setup();
    const gate = deferred<Uint8Array>();
    io.voxelizeResult = () => gate.promise;
    dropFile("model.glb", new Uint8Array([1]));
    await flush();
    const bar = must(document, "#build-bar", HTMLElement);
    expect(bar.hidden).toBe(false);
    io.lastOnProgress?.({ phase: "voxelize", done: 1, total: 2 });
    expect(status().text).toBe("model.glb: voxelizing on the GPU…");
    io.lastOnProgress?.({ phase: "colorBake", done: 0, total: 0 });
    expect(status().text).toBe("model.glb: baking voxel colors…");
    gate.resolve(new Uint8Array([2]));
    await flush();
    expect(bar.hidden).toBe(false); // completed bar lingers before auto-hiding
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("hides the build bar and reports the error when voxelization fails", async () => {
    const { io } = setup();
    io.voxelizeResult = () => Promise.reject(new Error("no triangles in mesh"));
    dropFile("empty.obj", new Uint8Array([1]));
    await flush();
    expect(must(document, "#build-bar", HTMLElement).hidden).toBe(true);
    expect(status()).toEqual({ text: "no triangles in mesh", isError: true });
  });

  it("wires the import button through the hidden file input", async () => {
    const { ui, io } = setup();
    const click = vi.spyOn(ui.file, "click").mockImplementation(() => undefined);
    ui.importBtn.click();
    expect(click).toHaveBeenCalledTimes(1);
    Object.defineProperty(ui.file, "files", {
      value: [new File([new Uint8Array([7])], "pick.vox")],
      configurable: true,
    });
    ui.file.dispatchEvent(new Event("change"));
    await flush();
    expect(io.decodeVoxCalls).toHaveLength(1);
  });

  it("survives a change event with no file selected", async () => {
    const { ui, io } = setup();
    ui.file.dispatchEvent(new Event("change"));
    await flush();
    expect(io.decodeVoxCalls).toHaveLength(0);
    expect(status()).toEqual({ text: "", isError: false });
  });
});

describe("exports", () => {
  function clickedDownloads(): string[] {
    const names: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      names.push(this.download);
    });
    return names;
  }

  it("snapshots, encodes, and downloads a .vox", async () => {
    const names = clickedDownloads();
    const { host, io } = setup();
    const snapshot = new Uint8Array([7, 7]);
    host.snapshotResult = () => Promise.resolve(snapshot);
    must(document, "#export-vox", HTMLButtonElement).click();
    await flush();
    expect(io.encodeVoxCalls).toEqual([snapshot]);
    expect(io.encodeCvoxCalls).toHaveLength(0);
    expect(names).toEqual(["voxel-web.vox"]);
    expect(URL.revokeObjectURL).toHaveBeenCalledTimes(1);
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("routes the cvox button to the cvox encoder", async () => {
    const names = clickedDownloads();
    const { io } = setup();
    must(document, "#export-cvox", HTMLButtonElement).click();
    await flush();
    expect(io.encodeCvoxCalls).toHaveLength(1);
    expect(io.encodeVoxCalls).toHaveLength(0);
    expect(names).toEqual(["voxel-web.cvox"]);
  });

  it("shows the encode bar (gather/write) and meters the gather", async () => {
    clickedDownloads();
    const { io } = setup();
    const gate = deferred<Uint8Array>();
    io.encodeVox = (_scene, onProgress) => {
      io.lastOnProgress = onProgress;
      return gate.promise;
    };
    must(document, "#export-vox", HTMLButtonElement).click();
    await flush();
    const bar = must(document, "#build-bar", HTMLElement);
    expect(bar.hidden).toBe(false);
    expect(bar.children).toHaveLength(2); // gather, write
    io.lastOnProgress?.({ phase: "gather", done: 5, total: 10 });
    expect(status().text).toBe("voxel-web.vox: gathering voxels…");
    gate.resolve(new Uint8Array([5]));
    await flush();
    expect(bar.hidden).toBe(false); // completed bar lingers before auto-hiding
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("reports a failed snapshot instead of downloading garbage", async () => {
    const names = clickedDownloads();
    const { host } = setup();
    host.snapshotResult = () => Promise.reject(new Error("worker gone"));
    must(document, "#export-vox", HTMLButtonElement).click();
    await flush();
    expect(names).toEqual([]);
    expect(status()).toEqual({ text: "worker gone", isError: true });
  });

  it("downloads the canvas image as png", () => {
    const names = clickedDownloads();
    const { ui } = setup();
    const image = new Blob(["png-bytes"]);
    ui.canvas.toBlob = (cb) => {
      cb(image);
    };
    must(document, "#export", HTMLButtonElement).click();
    expect(URL.createObjectURL).toHaveBeenCalledWith(image);
    expect(names).toEqual(["voxel-web.png"]);
  });

  it("reports a failed png capture", () => {
    const { ui } = setup();
    ui.canvas.toBlob = (cb) => {
      cb(null);
    };
    must(document, "#export", HTMLButtonElement).click();
    expect(status()).toEqual({ text: "png capture failed", isError: true });
  });
});

describe("camera mode selector", () => {
  const active = (): string[] =>
    [...document.querySelectorAll("#cam-orbit, #cam-fly")]
      .filter((b) => (b as HTMLElement).classList.contains("active"))
      .map((b) => b.id);

  it("boots with orbit active", () => {
    const { host } = setup();
    expect(active()).toEqual(["cam-orbit"]);
    expect(host.cameraModes).toEqual([]); // no host call until the user picks
  });

  it("clicking a mode tells the host and highlights exactly that button", () => {
    const { host } = setup();
    must(document, "#cam-orbit", HTMLButtonElement).click();
    expect(host.cameraModes).toEqual([CameraMode.Orbit]);
    expect(active()).toEqual(["cam-orbit"]);
    must(document, "#cam-fly", HTMLButtonElement).click();
    expect(host.cameraModes).toEqual([CameraMode.Orbit, CameraMode.Fly]);
    expect(active()).toEqual(["cam-fly"]);
  });

  it("a new scene load re-syncs the buttons to orbit; a re-derivation keeps the mode", async () => {
    const { ui } = setup();
    must(document, "#cam-fly", HTMLButtonElement).click();
    expect(active()).toEqual(["cam-fly"]);

    // Import a mesh (a new load) → the engine reset to the ambient orbit, so
    // the buttons follow.
    dropFile("model.glb", new Uint8Array([1]));
    await flush();
    expect(active()).toEqual(["cam-orbit"]);

    // Back to fly, then re-voxelize (a re-derivation) → the mode is kept.
    must(document, "#cam-fly", HTMLButtonElement).click();
    ui.res.value = "512";
    ui.res.dispatchEvent(new Event("change"));
    must(document, "#revoxelize", HTMLButtonElement).click();
    await flush();
    expect(active()).toEqual(["cam-fly"]);
  });
});

describe("input and stats wiring", () => {
  it("forwards keys and brush pointer events with pressure to the host", () => {
    const { ui, host } = setup({ editable: true });
    ui.canvas.setPointerCapture = () => undefined;
    ui.canvas.hasPointerCapture = () => false;
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyW", bubbles: true }));
    expect(host.keys.at(-1)).toEqual([KeyAction.Forward, true]);

    const down = new PointerEvent("pointerdown", { button: 2, pointerId: 1 });
    Object.defineProperty(down, "offsetX", { value: 5 });
    Object.defineProperty(down, "offsetY", { value: 6 });
    ui.canvas.dispatchEvent(down);
    // Radius/tool go via set_brush; the pointer event carries only x/y/pressure
    // (a mouse reports no pen pressure → full 1.0).
    expect(host.brushes).toEqual([[5, 6, 1]]);
    ui.canvas.dispatchEvent(new PointerEvent("pointerup", { button: 2, pointerId: 1 }));
    expect(host.brushEnds).toBe(1);
  });

  it("forwards the radius and strength sliders to set_brush", () => {
    const { ui, host } = setup({ editable: true });
    host.setBrushes.length = 0;
    ui.brushRadius.value = "9";
    ui.brushRadius.dispatchEvent(new Event("input"));
    expect(host.setBrushes.at(-1)?.[1]).toBe(9);
    ui.strength.value = "0.5";
    ui.strength.dispatchEvent(new Event("input"));
    expect(host.setBrushes.at(-1)?.[2]).toBe(0.5);
  });

  it("swallows brush strokes on a non-editable scene (brushEnd still flows)", () => {
    const { ui, host } = setup({ editable: false });
    ui.canvas.setPointerCapture = () => undefined;
    ui.canvas.hasPointerCapture = () => false;
    ui.canvas.dispatchEvent(new PointerEvent("pointerdown", { button: 2, pointerId: 1 }));
    ui.canvas.dispatchEvent(new PointerEvent("pointerup", { button: 2, pointerId: 1 }));
    expect(host.brushes).toEqual([]);
    expect(host.brushEnds).toBe(1);
  });

  it("coalesces observer bursts into one resize per frame", () => {
    const { ui, host } = setup();
    Object.defineProperty(ui.canvas, "clientWidth", { value: 300, configurable: true });
    Object.defineProperty(ui.canvas, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    // An interactive resize fires the observer repeatedly within one frame;
    // each worker-side resize reallocates the output texture, so only the
    // frame's final size may go through.
    resizeCallback?.();
    Object.defineProperty(ui.canvas, "clientWidth", { value: 320, configurable: true });
    resizeCallback?.();
    resizeCallback?.();
    expect(host.resizes).toEqual([]); // nothing until the frame
    expect(rafQueue).toHaveLength(1);
    rafQueue.shift()?.(0);
    expect(host.resizes).toEqual([[640, 400]]); // one resize, the latest size
    // The next burst schedules a fresh frame.
    resizeCallback?.();
    expect(rafQueue).toHaveLength(1);
  });

  it("routes host stats into the HUD cells", () => {
    const { host } = setup();
    host.statsCb?.({
      fps: 120,
      frameAvg: 8.3,
      frameMin: 8,
      frameMax: 9,
      frames: 5000,
      nodes: 1,
      leaves: 2,
      voxels: 3,
      undoDepth: 0,
      redoDepth: 0,
      truecolor: false,
      heapBytes: 96 * 2 ** 20,
    });
    expect(must(document, "#stat-fps", HTMLElement).textContent).toBe("120");
    expect(must(document, "#stat-frames", HTMLElement).textContent).toBe("5.0K");
  });

  it("routes both wasm-heap gauges into the HUD heap cell", () => {
    const { host, io } = setup();
    host.statsCb?.({
      fps: 60,
      frameAvg: 16,
      frameMin: 16,
      frameMax: 16,
      frames: 1,
      nodes: 1,
      leaves: 2,
      voxels: 3,
      undoDepth: 0,
      redoDepth: 0,
      truecolor: false,
      heapBytes: 128 * 2 ** 20,
    });
    io.heapCb?.(1.5 * 2 ** 30);
    expect(must(document, "#stat-heap", HTMLElement).textContent).toBe("r 128M · io 1.50G");
    io.heapCb?.(0); // the IO worker was recycled: the heap reads as freed
    expect(must(document, "#stat-heap", HTMLElement).textContent).toBe("r 128M · io 0M");
  });
});

describe("panel docks", () => {
  it("first run: every panel folded (the canvas is the landing experience)", () => {
    const { ui } = setup();
    expect(ui.panelEdit.hidden).toBe(true);
    expect(ui.panelScene.hidden).toBe(true);
    expect(ui.panelIo.hidden).toBe(true);
    expect(ui.panelStats.hidden).toBe(true);
    expect(ui.toggleEdit.getAttribute("aria-expanded")).toBe("false");
    expect(ui.toggleIo.getAttribute("aria-expanded")).toBe("false");
  });

  it("each toggle opens and closes only its own panel", () => {
    const { ui } = setup();
    ui.toggleIo.click();
    expect(ui.panelIo.hidden).toBe(false);
    expect(ui.toggleIo.getAttribute("aria-expanded")).toBe("true");
    ui.toggleEdit.click();
    expect(ui.panelEdit.hidden).toBe(false);
    // Independence: the others kept their state.
    expect(ui.panelIo.hidden).toBe(false);
    expect(ui.panelScene.hidden).toBe(true);
    ui.toggleEdit.click();
    expect(ui.panelEdit.hidden).toBe(true);
  });

  it("persists each panel's choice across sessions", () => {
    const first = setup();
    first.ui.toggleStats.click(); // open stats
    first.ui.toggleScene.click(); // open scene
    expect(JSON.parse(localStorage.getItem("voxel-web.panels") ?? "{}")).toEqual({
      edit: false,
      scene: true,
      io: false,
      stats: true,
      settings: false,
    });
    // A fresh boot (new DOM, same storage) restores the exact composition.
    mount();
    const second = setup();
    expect(second.ui.panelStats.hidden).toBe(false);
    expect(second.ui.panelScene.hidden).toBe(false);
    expect(second.ui.panelEdit.hidden).toBe(true);
  });

  it("controls in a folded panel still work by hotkey (the dock hides, not disables)", () => {
    const { ui, host } = setup({ editable: true });
    // The brush panel is folded by default — the hotkey must still reach it.
    expect(ui.panelEdit.hidden).toBe(true);
    host.setBrushes.length = 0;
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Digit3", bubbles: true }));
    expect(host.setBrushes.at(-1)?.[0]).toBe(BrushTool.Clay);
  });

  it("accent swatches swap the seed attribute, persist, and amber clears it", () => {
    delete document.documentElement.dataset["accent"];
    const first = setup();
    const [amber, teal] = first.ui.accentSwatches;
    expect(amber?.getAttribute("aria-pressed")).toBe("true");
    teal?.click();
    expect(document.documentElement.dataset["accent"]).toBe("teal");
    expect(teal?.getAttribute("aria-pressed")).toBe("true");
    expect(amber?.getAttribute("aria-pressed")).toBe("false");
    expect(localStorage.getItem("voxel-web.accent")).toBe("teal");
    // A fresh boot restores the stored accent.
    mount();
    delete document.documentElement.dataset["accent"];
    const second = setup();
    expect(document.documentElement.dataset["accent"]).toBe("teal");
    // Amber is the default: selecting it clears the override.
    second.ui.accentSwatches[0]?.click();
    expect(document.documentElement.dataset["accent"]).toBeUndefined();
  });

  it("the render background follows the theme (boot + every flip)", () => {
    delete document.documentElement.dataset["theme"];
    const { ui, host } = setup();
    // Boot sends the effective theme's sky endpoints.
    expect(host.backgrounds).toHaveLength(1);
    ui.themeToggle.click();
    expect(host.backgrounds).toHaveLength(2);
    // The flipped theme sends different endpoints.
    expect(host.backgrounds[1]).not.toEqual(host.backgrounds[0]);
  });

  it("the theme toggle overrides the OS, flips, and persists", () => {
    delete document.documentElement.dataset["theme"];
    const { ui } = setup();
    // No override yet: following the OS (whatever this environment reports).
    expect(document.documentElement.dataset["theme"]).toBeUndefined();
    const osLight = matchMedia("(prefers-color-scheme: light)").matches;
    const away = osLight ? "dark" : "light";
    const back = osLight ? "light" : "dark";
    ui.themeToggle.click(); // flips away from the effective OS mode
    expect(document.documentElement.dataset["theme"]).toBe(away);
    expect(localStorage.getItem("voxel-web.theme")).toBe(away);
    ui.themeToggle.click();
    expect(document.documentElement.dataset["theme"]).toBe(back);
    // A fresh boot restores the stored override.
    mount();
    delete document.documentElement.dataset["theme"];
    setup();
    expect(document.documentElement.dataset["theme"]).toBe(back);
  });

  it("the ui fragments into function islands inside the stage", () => {
    setup();
    const stage = must(document, "#ui", HTMLElement);
    for (const id of ["panel-edit", "panel-scene", "panel-io", "panel-stats", "camera-dock"]) {
      expect(stage.contains(must(document, `#${id}`, HTMLElement))).toBe(true);
    }
  });

  it("status and the build bar live outside every toggleable panel", () => {
    const { ui } = setup();
    const status = must(document, "#status", HTMLElement);
    const bar = must(document, "#build-bar", HTMLElement);
    for (const panel of [ui.panelEdit, ui.panelScene, ui.panelIo, ui.panelStats]) {
      expect(panel.contains(status)).toBe(false);
      expect(panel.contains(bar)).toBe(false);
    }
    expect(must(document, "#ui", HTMLElement).contains(status)).toBe(false);
  });
});

describe("stroke history (undo/redo)", () => {
  /** A stats sample carrying only what the history UI reads. */
  function statsWithDepths(undoDepth: number, redoDepth: number): HudStats {
    return {
      fps: 60,
      frameAvg: 16,
      frameMin: 16,
      frameMax: 16,
      frames: 1,
      nodes: 1,
      leaves: 2,
      voxels: 3,
      undoDepth,
      redoDepth,
      truecolor: false,
      heapBytes: 0,
    };
  }

  it("buttons start disabled, enable with depth, and show the count", () => {
    const { host, ui } = setup();
    expect(ui.undoBtn.disabled).toBe(true);
    expect(ui.redoBtn.disabled).toBe(true);
    host.statsCb?.(statsWithDepths(3, 1));
    expect(ui.undoBtn.disabled).toBe(false);
    expect(ui.undoBtn.textContent).toBe("undo ×3");
    expect(ui.redoBtn.disabled).toBe(false);
    expect(ui.redoBtn.textContent).toBe("redo ×1");
    host.statsCb?.(statsWithDepths(0, 0));
    expect(ui.undoBtn.disabled).toBe(true);
    expect(ui.undoBtn.textContent).toBe("undo");
    expect(ui.redoBtn.disabled).toBe(true);
  });

  it("clicking the buttons sends undo/redo to the host", () => {
    const { host, ui } = setup();
    host.statsCb?.(statsWithDepths(2, 2));
    ui.undoBtn.click();
    ui.undoBtn.click();
    ui.redoBtn.click();
    expect(host.undos).toBe(2);
    expect(host.redos).toBe(1);
  });

  it("Cmd+Z / Shift+Cmd+Z route through the input carve-out to the host", () => {
    const { host } = setup();
    window.dispatchEvent(
      new KeyboardEvent("keydown", { code: "KeyZ", metaKey: true, bubbles: true, cancelable: true }),
    );
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "KeyZ",
        metaKey: true,
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(host.undos).toBe(1);
    expect(host.redos).toBe(1);
  });

  it("a scene install zeroes the depths without waiting for a stats tick", async () => {
    const { host, io, ui } = setup();
    host.statsCb?.(statsWithDepths(5, 2));
    expect(ui.undoBtn.textContent).toBe("undo ×5");
    dropFile("model.glb", new Uint8Array([1, 2, 3]));
    await flush();
    expect(io.voxelizeCalls.length).toBe(1);
    expect(ui.undoBtn.disabled).toBe(true);
    expect(ui.undoBtn.textContent).toBe("undo");
    expect(ui.redoBtn.disabled).toBe(true);
  });
});

describe("canvas helpers", () => {
  it("measureCanvas clamps DPR to 2 and floors at one pixel", () => {
    const canvas = document.createElement("canvas");
    Object.defineProperty(window, "devicePixelRatio", { value: 3, configurable: true });
    expect(measureCanvas(canvas)).toEqual({ width: 1, height: 1 }); // 0-sized layout
    Object.defineProperty(canvas, "clientWidth", { value: 400 });
    Object.defineProperty(canvas, "clientHeight", { value: 250 });
    expect(measureCanvas(canvas)).toEqual({ width: 800, height: 500 });
  });

  it("resetCanvas swaps in a fresh element with the same identity", () => {
    const old = must(document, "#view", HTMLCanvasElement);
    const fresh = resetCanvas(old);
    expect(fresh).not.toBe(old);
    expect(fresh.id).toBe("view");
    expect(fresh.getAttribute("aria-label")).toBe("voxel renderer output");
    expect(must(document, "#view", HTMLCanvasElement)).toBe(fresh);
    expect(old.isConnected).toBe(false);
  });
});

describe("main boot ladder", () => {
  function overlay(): { hidden: boolean; title: string; body: string } {
    return {
      hidden: must(document, "#overlay", HTMLElement).hidden,
      title: must(document, "#overlay-title", HTMLElement).textContent,
      body: must(document, "#overlay-body", HTMLElement).textContent,
    };
  }

  function makeDeps(): { deps: BootDeps; worker: FakeHost; local: FakeHost; io: FakeIo } {
    const worker = new FakeHost();
    const local = new FakeHost();
    const io = new FakeIo();
    return {
      worker,
      local,
      io,
      deps: {
        createWorkerHost: () => worker,
        createLocalHost: () => local,
        createIo: () => io,
      },
    };
  }

  function withGpu(): void {
    Object.defineProperty(navigator, "gpu", { value: {}, configurable: true });
  }

  afterEach(() => {
    Reflect.deleteProperty(navigator, "gpu");
  });

  it("shows the WebGPU overlay and builds nothing when the API is missing", async () => {
    expect("gpu" in navigator).toBe(false); // the environment really lacks it
    await main({
      createWorkerHost: () => {
        throw new Error("must not construct");
      },
      createLocalHost: () => {
        throw new Error("must not construct");
      },
      createIo: () => {
        throw new Error("must not construct");
      },
    });
    expect(overlay()).toMatchObject({ hidden: false, title: "WebGPU unavailable" });
  });

  it("starts the worker host on the page canvas with the default scene", async () => {
    withGpu();
    const { deps, worker, local, io } = makeDeps();
    await main(deps);
    expect(worker.startCalls).toHaveLength(1);
    expect(worker.startCalls[0]?.canvas).toBe(must(document, "#view", HTMLCanvasElement));
    expect(worker.startCalls[0]?.opts).toEqual({ res: 128, fixture: "gyroid" });
    expect(local.startCalls).toHaveLength(0);
    expect(overlay().hidden).toBe(true);
    expect(status()).toEqual({ text: "", isError: false });
    // run() is wired: a dropped model now reaches the injected IO.
    dropFile("model.glb", new Uint8Array([1, 2, 3]));
    await flush();
    expect(io.voxelizeCalls).toHaveLength(1);
  });

  it("falls back to the local host on a fresh canvas when the worker start fails", async () => {
    withGpu();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { deps, worker, local } = makeDeps();
    const spentCanvas = must(document, "#view", HTMLCanvasElement);
    worker.startResult = () => Promise.reject(new Error("OffscreenCanvas unsupported"));
    await main(deps);
    expect(warn).toHaveBeenCalledOnce();
    // The failed worker host must be released, not left idling with its wasm
    // instance for the session.
    expect(worker.disposed).toBe(1);
    const freshCanvas = must(document, "#view", HTMLCanvasElement);
    expect(freshCanvas).not.toBe(spentCanvas); // the transferred canvas is spent
    expect(local.startCalls[0]?.canvas).toBe(freshCanvas);
    expect(overlay().hidden).toBe(true);
    expect(status()).toEqual({ text: "", isError: false });
  });

  it("shows the failure overlay when both hosts refuse to start", async () => {
    withGpu();
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { deps, worker, local } = makeDeps();
    worker.startResult = () => Promise.reject(new Error("worker dead"));
    local.startResult = () => Promise.reject(new Error("no adapter on main either"));
    await main(deps);
    expect(overlay()).toEqual({
      hidden: false,
      title: "failed to start",
      body: "no adapter on main either",
    });
  });
});
