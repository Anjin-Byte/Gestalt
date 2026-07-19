// The curated portfolio demo set, as the shell sees it.
//
// The pack step (tools/demo-pack, run by `make demos`) emits
// web/src/assets/demos/{index.json, <id>.glb.gz}. This module joins each index
// entry's per-model bake options to its packed blob's hashed Vite URL. Both are
// read through `import.meta.glob`, so a checkout that has not run `make demos`
// yet simply yields an empty list — the picker shows no demos and the boot path
// is untouched (docs/design/demo-assets.md §6, §8).

export interface Demo {
  readonly id: string;
  readonly title: string;
  /** Hashed URL of the packed, gzipped `.glb.gz` (fetched + inflated on load). */
  readonly url: string;
  /** Hashed URL of the gallery thumbnail, or null → the gallery draws a
   * placeholder card (thumbnails are hand-authored and optional). */
  readonly thumbnail: string | null;
  readonly res: number;
  readonly zUp: boolean;
  readonly truecolor: boolean;
  readonly gpuBake: boolean;
  readonly alphaMode: "opaque" | "mask" | "blend";
  /** Display attribution (CC-BY etc.), or null when none is recorded. */
  readonly attribution: string | null;
}

/** One row of the generated index.json (the pack tool's shell contract). */
interface RawDemo {
  readonly id: string;
  readonly title: string;
  readonly file: string;
  readonly thumb: string | null;
  readonly res: number;
  readonly zUp: boolean;
  readonly truecolor: boolean;
  readonly gpuBake: boolean;
  readonly alphaMode: Demo["alphaMode"];
  readonly attribution: string | null;
}

// The generated shell manifest — a single file, absent until `make demos`. Glob
// (not a static import) so its absence is an empty map, not a build error.
const INDEX = import.meta.glob<RawDemo[]>("./assets/demos/index.json", {
  eager: true,
  import: "default",
});

// Each packed blob / thumbnail resolved to a hashed asset URL string.
const BLOB_URLS = import.meta.glob<string>("./assets/demos/*.glb.gz", {
  eager: true,
  query: "?url",
  import: "default",
});
const THUMB_URLS = import.meta.glob<string>("./assets/demos/*.thumb.webp", {
  eager: true,
  query: "?url",
  import: "default",
});

function urlIn(map: Record<string, string>, file: string): string | undefined {
  for (const [key, url] of Object.entries(map)) {
    if (key.endsWith(`/${file}`) || key === `./assets/demos/${file}`) {
      return url;
    }
  }
  return undefined;
}

/** The demos with a packed blob present, in manifest order. Empty when the pack
 * step has not run — every caller degrades to "no demos" gracefully. */
export function listDemos(): Demo[] {
  const index = Object.values(INDEX)[0];
  if (!Array.isArray(index)) {
    return [];
  }
  const demos: Demo[] = [];
  for (const raw of index) {
    const url = urlIn(BLOB_URLS, raw.file);
    if (url === undefined) {
      continue; // index lists it but its blob is not on disk — skip quietly
    }
    demos.push({
      id: raw.id,
      title: raw.title,
      url,
      thumbnail: raw.thumb !== null ? (urlIn(THUMB_URLS, raw.thumb) ?? null) : null,
      res: raw.res,
      zUp: raw.zUp,
      truecolor: raw.truecolor,
      gpuBake: raw.gpuBake,
      alphaMode: raw.alphaMode,
      attribution: raw.attribution,
    });
  }
  return demos;
}

/** Fetches a packed demo and returns the raw `.glb` bytes. The blobs are stored
 * gzipped, but hosts differ on how they serve a `.gz` file: Vite (and most CDNs)
 * set `Content-Encoding: gzip`, so the browser inflates transparently and
 * `fetch` already hands us glTF; a plain static host serves the compressed bytes
 * untouched. We detect the gzip magic (0x1f 0x8b) and inflate only when the
 * bytes are still compressed — inflating a browser-inflated body would double-
 * decode and fail (design §4.5). */
export async function fetchDemoMesh(url: string): Promise<Uint8Array<ArrayBuffer>> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`demo fetch failed (${res.status} ${res.statusText})`);
  }
  const bytes = new Uint8Array(await res.arrayBuffer());
  if (bytes.length === 0) {
    throw new Error("demo fetch: empty response");
  }
  const stillGzipped = bytes[0] === 0x1f && bytes[1] === 0x8b;
  if (!stillGzipped) {
    return bytes; // the browser already inflated it (Content-Encoding: gzip)
  }
  const stream = new Response(bytes).body;
  if (stream === null) {
    throw new Error("demo inflate: no stream for the gzipped body");
  }
  const inflated = stream.pipeThrough(new DecompressionStream("gzip"));
  return new Uint8Array(await new Response(inflated).arrayBuffer());
}
