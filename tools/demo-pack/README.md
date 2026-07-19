# demo-pack

The pack step for the curated portfolio demo set: turns the committed source
`.glb` in [`demo-assets/src/`](../../demo-assets/) into loader-envelope-safe,
texture-resized, gzipped outputs the web shell ships.

It is a **standalone tool on purpose.** It pulls in `gltf-transform` and `sharp`
(heavy, specialized, native) — dependencies that have no business inside the web
shell's `package.json` or the voxel crates. Nothing else in the repo imports it;
`make demos` invokes it and the web build consumes its output.

Design reference: [`docs/design/demo-assets.md`](../../docs/design/demo-assets.md)
— §3 (loader envelope), §4 (the recipe), §7 (manifest).

## What it does, per model

1. **Envelope check** — reject any source that requires/uses Draco, meshopt,
   KTX2/Basis, or (until verified) mesh quantization. Our importer is the Rust
   `gltf` + `image` crates; those extensions would simply fail to parse (§3).
2. **Scene 0 only** — the importer reads the default scene; extra scenes are
   dropped (§7).
3. **Strip** everything the voxel bake never samples: animation, skins, morph
   targets, and per-vertex normals / tangents / colours. UV sets stay (tiny; the
   textures need them).
4. **Texture resize + re-encode** — *the* lever (§4.1). Every texture is capped
   to the model's `textureMax` and re-encoded: JPEG at `textureQuality` for
   opaque materials, PNG for the base-colour texture of mask/blend materials (to
   keep the alpha the cutout needs).
5. **Geometry hygiene** — `weld` coincident verts, `dedup` shared data, `prune`
   orphans.
6. **Gzip at rest** (`<id>.glb.gz`) — inflated in the shell via
   `DecompressionStream`.

## Usage

```
node pack.mjs                  # pack every manifest entry (incremental)
node pack.mjs littlest_tokyo   # pack only the named demo(s)
node pack.mjs --force          # repack even when outputs are up to date
node pack.mjs --inspect        # report each source (envelope + textures), no output
node pack.mjs --out <dir>      # override the output directory
```

Or via the repo Makefile:

```
make demos
```

Incremental: an output is skipped when it is newer than both its source and the
manifest. Exit code is non-zero if any requested demo fails (bad envelope,
missing source), so `make demos` fails the build loudly.

## Input / output

- **Input:** [`demo-assets/manifest.json`](../../demo-assets/manifest.json) and
  the `src/*.glb` it points at.
- **Output:** `web/src/assets/demos/<id>.glb.gz` — **generated, gitignored**. The
  Vite build hashes these into `web/dist/` via `?url` imports.

## Dependencies

`@gltf-transform/{core,functions,extensions}` for the glTF surgery and `sharp`
for texture resize/re-encode. `node_modules/` is gitignored; the lockfile is
committed. `make demos` runs `npm install` on first use.

> A cosmetic `objc[...] Class GNotificationCenterDelegate is implemented in
> both ...libvips...` warning may print on macOS — a duplicate-load notice from
> sharp's fallback dependency, not an error. The pack always uses the real sharp
> encoder we pass in.
