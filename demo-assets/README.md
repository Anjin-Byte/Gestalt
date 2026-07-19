# Curated portfolio demo set

A small, hand-picked set of models shipped with the **web app for the portfolio
site** — the "click a demo, watch it voxelize" experience. This is deliberately
**separate from the top-level `models/` directory**, which is a large, gitignored
scratch dump used for debugging the importer. These are the ones we stand behind
and ship.

Design reference: [`docs/design/demo-assets.md`](../docs/design/demo-assets.md)
(the decision record — transport choice, loader envelope, bake cache, manifest
schema). The pack step lives in [`tools/demo-pack`](../tools/demo-pack).

## ⚠️ Licenses are UNVERIFIED

Every model here was promoted out of the debug dump and its license has **not**
been confirmed. Before this set ships to the public portfolio site, each
`manifest.json` entry's `license.status` must read `"verified"` with a confirmed
author, URL, and SPDX identifier — and the attribution must be surfaced in the
demos UI (design §7, §10.4).

Author + source below are from each model's **embedded glTF metadata**
(`asset.extras`), cross-checked against the live model pages, and are populated
in `manifest.json` → `license.{author,url,attribution}`.

| id | author | source |
|---|---|---|
| `littlest_tokyo` | glenatron | [Littlest Tokyo](https://sketchfab.com/models/94b24a60dc1b48248de50bf087c0f042) |
| `bath_day` | Stan.St | [Bath day](https://sketchfab.com/3d-models/bath-day-c1b0d6edd934423d965e3cd032eb358e) |
| `nemetona` | JOJObrush | [Nemetona_NatureBeauty](https://sketchfab.com/3d-models/nemetona-naturebeauty-c338180707ce4b7c9bb4d1c6d24843c0) |
| `steampunk_camera` | lumoize | [Steampunk Camera](https://sketchfab.com/3d-models/steampunk-camera-a2210a0ba6834141af3bf83ee1e03f07) |
| `venice_mask` | DailyArt | [Venice Mask](https://sketchfab.com/3d-models/venice-mask-4aace12762ee44cf97d934a6ced12e65) |

**Licensing not carried in the manifest (per owner), but noted here:** the
embedded metadata reports all five as CC-BY-4.0 **except `venice_mask`, which is
CC-BY-NC-4.0 (NonCommercial)** — confirmed acceptable, as this portfolio use is
non-commercial. CC-BY/CC-BY-NC still expect the credit surfaced in the UI (it is,
on the gallery cards) and that modifications be indicated (these are voxelized —
a heavy modification).

## Layout

```
demo-assets/
  manifest.json          # one entry per demo (schema: design §7) — TRACKED
  src/                   # source-of-truth .glb, committed raw — TRACKED
    littlest_tokyo.glb
    steampunk_camera.glb
    venice_mask.glb
    nemetona.glb
    bath_day.glb
  README.md              # this file

web/src/assets/demos/    # packed .glb.gz output — GENERATED, gitignored
```

The raw sources (~102 MB total) are committed here directly. The packed,
texture-resized, gzipped outputs the app actually ships are **generated** by the
pack step into `web/src/assets/demos/` and are not committed — they rebuild
deterministically from these sources + the manifest.

## Packing

```
make demos          # incremental: repacks only what changed
```

Runs [`tools/demo-pack`](../tools/demo-pack), which resizes textures to the
bake-resolvable budget, strips everything the voxel bake never reads, gzips at
rest, and refuses any source that falls outside the importer's loader envelope
(no Draco / meshopt / KTX2). `make web`, `make web-setup`, and `make web-dist`
all run this first, so the shell always has fresh packed demos.

### Current packed sizes (wire, gzipped)

| id | source | packed .glb.gz | notes |
|---|---:|---:|---|
| `steampunk_camera` | 15 MiB | 1.6 MiB | 9 textures → JPEG, big win |
| `bath_day` | 10 MiB | 3.8 MiB | mask/blend → PNG base colour kept |
| `littlest_tokyo` | 13 MiB | 4.4 MiB | 4096² texture capped to 1024 |
| `nemetona` | 26 MiB | 4.6 MiB | **geometry-bound** — meshopt candidate (design §5 / D3) |
| `venice_mask` | 38 MiB | 6.7 MiB | 15 textures; **tuning candidate** (`textureMax: 512`) |

`venice_mask` and `nemetona` sit above the design's 1–4 MB target and are the two
obvious tuning levers if wire size matters: drop `venice_mask`'s `textureMax` to
512, and adopt meshopt for `nemetona` (rollout D3).

## Adding a demo

1. Drop the source `.glb` in `src/` (a single-scene, core-spec glTF — the pack
   step keeps only scene 0 and rejects compressed-geometry extensions).
2. Add a `manifest.json` entry (copy an existing one; see design §7 for each
   field). Pick `res`, `textureMax`, and `alphaMode` **by eye** for that model.
3. Fill in the real license — do not ship `status: "unverified"`.
4. `make demos` and open the app; check the silhouette reads as sculpture, not
   soup, at the chosen `res`.
