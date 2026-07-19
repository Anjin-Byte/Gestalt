#!/usr/bin/env node
// demo-pack — the curated portfolio demo set's pack step.
//
// Turns the committed source `.glb` in demo-assets/src/ into loader-envelope-safe,
// texture-resized, gzipped outputs the web shell ships. This is a *standalone*
// tool by design: it depends on gltf-transform + sharp (heavy, specialized) and
// must not leak those into the web shell or the voxel crates
// (docs/design/demo-assets.md — §3 envelope, §4 recipe, §7 manifest).
//
// Usage:
//   node pack.mjs                 pack every manifest entry (incremental)
//   node pack.mjs <id> [<id>…]    pack only the named demos
//   node pack.mjs --force         repack even when outputs are up to date
//   node pack.mjs --inspect       report each source (envelope + textures/meshes), no output
//   node pack.mjs --out <dir>     override the output directory
//
// Exit code is non-zero if any requested demo fails (a bad envelope, a missing
// source, an over-budget bake) — so `make demos` fails the build loudly.

import { gzipSync } from "node:zlib";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import {
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

import { NodeIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { dedup, prune, textureCompress, weld } from "@gltf-transform/functions";

const require = createRequire(import.meta.url);
const sharp = require("sharp");

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..", "..");
const DEMO_DIR = join(REPO_ROOT, "demo-assets");
const SRC_DIR = join(DEMO_DIR, "src");
const MANIFEST = join(DEMO_DIR, "manifest.json");
const DEFAULT_OUT = join(REPO_ROOT, "web", "src", "assets", "demos");

// Extensions our voxel importer (Rust `gltf` + `image`) cannot parse. A source
// that requires/uses any of these would fail to load in the app, so we reject it
// here rather than ship a demo that never opens (design §3).
const ENVELOPE_REJECT = new Map([
  ["KHR_draco_mesh_compression", "Draco geometry — no Rust decoder in the wasm build"],
  ["EXT_meshopt_compression", "meshopt geometry — needs a JS-side transcode we don't ship"],
  ["KHR_texture_basisu", "KTX2/Basis textures — importer reads PNG/JPEG only"],
  ["KHR_mesh_quantization", "quantized accessors — UNVERIFIED against our loader (design §3); gate before allowing"],
]);

const c = {
  dim: (s) => `\x1b[2m${s}\x1b[0m`,
  red: (s) => `\x1b[31m${s}\x1b[0m`,
  green: (s) => `\x1b[32m${s}\x1b[0m`,
  yellow: (s) => `\x1b[33m${s}\x1b[0m`,
  bold: (s) => `\x1b[1m${s}\x1b[0m`,
};

const fmtBytes = (n) => {
  const u = ["B", "KiB", "MiB", "GiB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
};

/** Reads the JSON chunk out of a GLB container without a full parse — used for
 * the envelope check and for inspection, so a source we can't parse still yields
 * a clear diagnosis instead of a decoder stack trace. */
function readGlbJson(buf) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const magic = dv.getUint32(0, true);
  if (magic !== 0x46546c67) throw new Error("not a GLB (bad magic)");
  let offset = 12;
  while (offset < buf.byteLength) {
    const len = dv.getUint32(offset, true);
    const type = dv.getUint32(offset + 4, true);
    const start = offset + 8;
    if (type === 0x4e4f534a) {
      // 'JSON'
      return JSON.parse(Buffer.from(buf.buffer, buf.byteOffset + start, len).toString("utf8"));
    }
    offset = start + len + ((4 - (len % 4)) % 4);
  }
  throw new Error("GLB has no JSON chunk");
}

/** Returns the reject reasons (empty ⇒ inside the envelope). */
function envelopeViolations(gltfJson) {
  const names = new Set([
    ...(gltfJson.extensionsRequired ?? []),
    ...(gltfJson.extensionsUsed ?? []),
  ]);
  const out = [];
  for (const [ext, why] of ENVELOPE_REJECT) {
    if (names.has(ext)) out.push(`${ext} — ${why}`);
  }
  return out;
}

function loadManifest() {
  if (!existsSync(MANIFEST)) {
    fail(`no manifest at ${rel(MANIFEST)}`);
  }
  const parsed = JSON.parse(readFileSync(MANIFEST, "utf8"));
  const demos = parsed.demos ?? parsed;
  if (!Array.isArray(demos)) fail("manifest must be { demos: [...] } or a bare array");
  return demos;
}

const rel = (p) => p.replace(`${REPO_ROOT}/`, "");

function fail(msg) {
  console.error(c.red(`demo-pack: ${msg}`));
  process.exit(1);
}

// ── inspect ────────────────────────────────────────────────────────────────

async function inspect(io) {
  const files = existsSync(SRC_DIR)
    ? readdirSync(SRC_DIR).filter((f) => f.toLowerCase().endsWith(".glb")).sort()
    : [];
  if (files.length === 0) fail(`no .glb sources in ${rel(SRC_DIR)}`);
  console.log(c.bold(`\nInspecting ${files.length} source(s) in ${rel(SRC_DIR)}\n`));
  let anyFail = false;
  for (const file of files) {
    const path = join(SRC_DIR, file);
    const buf = readFileSync(path);
    let json;
    try {
      json = readGlbJson(buf);
    } catch (e) {
      console.log(`${c.red("✗")} ${c.bold(file)} — ${e.message}`);
      anyFail = true;
      continue;
    }
    const violations = envelopeViolations(json);
    const scenes = json.scenes?.length ?? 0;
    const materials = json.materials ?? [];
    const alphaModes = new Set(materials.map((m) => m.alphaMode ?? "OPAQUE"));
    const badge = violations.length ? c.red("REJECT") : c.green("ok");
    console.log(`${c.bold(file)}  ${badge}  ${c.dim(fmtBytes(buf.byteLength))}`);
    console.log(
      c.dim(
        `  scenes ${scenes}${scenes > 1 ? " (only scene 0 ships)" : ""}` +
          ` · meshes ${json.meshes?.length ?? 0} · materials ${materials.length}` +
          ` · images ${json.images?.length ?? 0}`,
      ),
    );
    console.log(c.dim(`  alphaModes {${[...alphaModes].join(", ")}}`));
    if (json.extensionsUsed?.length) {
      console.log(c.dim(`  extensionsUsed [${json.extensionsUsed.join(", ")}]`));
    }
    // Texture dimensions (via a full parse; skipped if the envelope rejected it).
    if (!violations.length) {
      try {
        const doc = await io.readBinary(new Uint8Array(buf));
        const dims = doc
          .getRoot()
          .listTextures()
          .map((t) => {
            const size = t.getSize();
            return `${size ? `${size[0]}×${size[1]}` : "?"} ${t.getMimeType().replace("image/", "")}`;
          });
        if (dims.length) console.log(c.dim(`  textures: ${dims.join(", ")}`));
      } catch (e) {
        console.log(c.yellow(`  (texture probe failed: ${e.message})`));
      }
    }
    for (const v of violations) console.log(`  ${c.red("↳")} ${v}`);
    if (violations.length) anyFail = true;
    console.log();
  }
  return anyFail ? 1 : 0;
}

// ── pack ───────────────────────────────────────────────────────────────────

/** True when the output is newer than both its source and the manifest — the
 * incremental skip. */
function upToDate(srcPath, outPath) {
  if (!existsSync(outPath)) return false;
  const out = statSync(outPath).mtimeMs;
  return out >= statSync(srcPath).mtimeMs && out >= statSync(MANIFEST).mtimeMs;
}

async function packOne(io, entry, outDir, force) {
  const id = entry.id;
  const srcPath = resolve(DEMO_DIR, entry.source);
  if (!existsSync(srcPath)) throw new Error(`${id}: source not found: ${rel(srcPath)}`);
  const outPath = join(outDir, `${id}.glb.gz`);

  if (!force && upToDate(srcPath, outPath)) {
    console.log(`${c.dim("·")} ${id} ${c.dim("up to date")}`);
    return { id, skipped: true };
  }

  const srcBuf = readFileSync(srcPath);
  const violations = envelopeViolations(readGlbJson(srcBuf));
  if (violations.length) {
    throw new Error(`${id}: outside loader envelope:\n    ${violations.join("\n    ")}`);
  }

  const doc = await io.readBinary(new Uint8Array(srcBuf));
  const root = doc.getRoot();

  // 1. Scene 0 only — the importer reads the default scene (design §7).
  const scenes = root.listScenes();
  if (scenes.length > 1) {
    root.setDefaultScene(scenes[0]);
    for (const s of scenes.slice(1)) s.dispose();
  }

  // 2. Strip everything the voxel bake never samples (design §4.2): animation,
  //    rigging, morphs, and per-vertex normals/tangents/colours. UVs stay — the
  //    textures need them and they are tiny next to the pixels.
  for (const a of root.listAnimations()) a.dispose();
  for (const s of root.listSkins()) s.dispose();
  for (const mesh of root.listMeshes()) {
    for (const prim of mesh.listPrimitives()) {
      for (const t of prim.listTargets()) t.dispose();
      for (const semantic of prim.listSemantics()) {
        if (semantic === "POSITION" || semantic.startsWith("TEXCOORD_")) continue;
        prim.setAttribute(semantic, null);
      }
    }
  }

  // 3. Texture resize + re-encode — *the* lever (design §4.1). Base colour keeps
  //    PNG when the model needs its alpha (mask/blend); everything else is JPEG.
  const max = entry.textureMax;
  const q = entry.textureQuality ?? 80;
  const resize = [max, max];
  if ((entry.alphaMode ?? "opaque") === "opaque") {
    await doc.transform(textureCompress({ encoder: sharp, targetFormat: "jpeg", quality: q, resize }));
  } else {
    await doc.transform(
      textureCompress({ encoder: sharp, targetFormat: "png", resize, slots: /baseColorTexture/ }),
      textureCompress({
        encoder: sharp,
        targetFormat: "jpeg",
        quality: q,
        resize,
        slots: /^(?!baseColorTexture$)/,
      }),
    );
  }

  // 4. Geometry hygiene: weld coincident verts, dedup shared data, prune orphans.
  await doc.transform(weld(), dedup(), prune());

  // 5. Emit GLB, gzip at rest (design §4.5) — inflated in the shell via
  //    DecompressionStream.
  const glb = await io.writeBinary(doc);
  const gz = gzipSync(Buffer.from(glb), { level: 9 });
  mkdirSync(outDir, { recursive: true });
  writeFileSync(outPath, gz);

  const ratio = srcBuf.byteLength / gz.byteLength;
  console.log(
    `${c.green("✓")} ${c.bold(id.padEnd(18))} ` +
      `${c.dim(fmtBytes(srcBuf.byteLength).padStart(9))} → ` +
      `${fmtBytes(glb.byteLength).padStart(9)} glb, ` +
      `${c.bold(fmtBytes(gz.byteLength).padStart(9))} gz ` +
      `${c.dim(`(${ratio.toFixed(1)}× smaller)`)}`,
  );
  return { id, srcBytes: srcBuf.byteLength, glbBytes: glb.byteLength, gzBytes: gz.byteLength };
}

async function pack(io, ids, outDir, force) {
  const demos = loadManifest();
  const selected = ids.length ? demos.filter((d) => ids.includes(d.id)) : demos;
  if (ids.length) {
    const known = new Set(demos.map((d) => d.id));
    for (const id of ids) if (!known.has(id)) fail(`unknown demo id: ${id}`);
  }
  if (selected.length === 0) fail("manifest has no demos");
  console.log(c.bold(`\nPacking ${selected.length} demo(s) → ${rel(outDir)}\n`));

  const results = [];
  let failed = 0;
  for (const entry of selected) {
    try {
      results.push(await packOne(io, entry, outDir, force));
      await syncThumbnail(entry, outDir, force);
    } catch (e) {
      console.error(`${c.red("✗")} ${e.message}`);
      failed += 1;
    }
  }

  const packed = results.filter((r) => !r.skipped);
  if (packed.length) {
    const totalGz = packed.reduce((s, r) => s + r.gzBytes, 0);
    console.log(c.dim(`\n  shipped total: ${fmtBytes(totalGz)} across ${packed.length} demo(s)`));
  }

  writeShellIndex(demos, outDir);

  if (failed) fail(`${failed} demo(s) failed`);
  console.log(c.green("\ndemos packed.\n"));
  return 0;
}

/** Resizes a hand-authored thumbnail to a gallery-sized WebP next to the packed
 * blob. Thumbnails are OPTIONAL (design: hand-authored, added over time) — a
 * missing source is a quiet skip so the gallery falls back to a placeholder
 * card. Runs independent of the .glb incremental skip: a thumbnail can be added
 * without touching the source model. */
async function syncThumbnail(entry, outDir, force) {
  if (!entry.thumbnail) return;
  const srcThumb = resolve(DEMO_DIR, entry.thumbnail);
  if (!existsSync(srcThumb)) {
    console.log(
      c.dim(`  · ${entry.id} thumbnail not found (${rel(srcThumb)}) — gallery placeholder`),
    );
    return;
  }
  const outThumb = join(outDir, `${entry.id}.thumb.webp`);
  if (!force && existsSync(outThumb) && statSync(outThumb).mtimeMs >= statSync(srcThumb).mtimeMs) {
    return; // up to date
  }
  mkdirSync(outDir, { recursive: true });
  await sharp(srcThumb)
    .resize(512, 512, { fit: "inside", withoutEnlargement: true })
    .webp({ quality: 82 })
    .toFile(outThumb);
  console.log(c.dim(`  · ${entry.id} thumbnail → ${fmtBytes(statSync(outThumb).size)} webp`));
}

/** Writes the shell-facing index next to the packed blobs: the subset of the
 * manifest the web picker needs (per-model bake options + attribution + the
 * optional thumbnail), for every demo whose packed output exists. The shell
 * joins each entry to its `<id>.glb.gz` / `<id>.thumb.webp` URLs via Vite globs
 * (web/src/demos.ts). Regenerated every run so it never drifts from disk. */
function writeShellIndex(demos, outDir) {
  const entries = demos
    .filter((d) => existsSync(join(outDir, `${d.id}.glb.gz`)))
    .map((d) => ({
      id: d.id,
      title: d.title ?? d.id,
      file: `${d.id}.glb.gz`,
      thumb: existsSync(join(outDir, `${d.id}.thumb.webp`)) ? `${d.id}.thumb.webp` : null,
      res: d.res,
      zUp: d.zUp ?? false,
      truecolor: d.truecolor ?? true,
      gpuBake: d.gpuBake ?? true,
      alphaMode: d.alphaMode ?? "opaque",
      attribution: d.license?.attribution ?? null,
    }));
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "index.json"), `${JSON.stringify(entries, null, 2)}\n`);
  console.log(c.dim(`  wrote index.json (${entries.length} demo(s))`));
}

// ── main ─────────────────────────────────────────────────────────────────────

async function main() {
  const argv = process.argv.slice(2);
  let outDir = DEFAULT_OUT;
  let force = false;
  let doInspect = false;
  const ids = [];
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--out") {
      outDir = resolve(argv[++i] ?? fail("--out needs a path"));
    } else if (a === "--force") {
      force = true;
    } else if (a === "--inspect") {
      doInspect = true;
    } else if (a.startsWith("-")) {
      fail(`unknown flag: ${a}`);
    } else {
      ids.push(a);
    }
  }

  // Register every core-spec extension so material variants read cleanly; the
  // compression/KTX2 extensions are caught earlier by the envelope check, before
  // any decoder is asked for.
  const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);

  const code = doInspect ? await inspect(io) : await pack(io, ids, outDir, force);
  process.exit(code);
}

main().catch((e) => fail(e.stack ?? String(e)));
