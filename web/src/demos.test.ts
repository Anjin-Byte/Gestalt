// demos.ts is the shell's view of the packed portfolio set. The interesting,
// environment-independent behaviour is the fetch+inflate seam and the structural
// contract of listDemos(); the actual demo list depends on whether `make demos`
// has run, so those tests assert shape, not count.
import { gzipSync } from "node:zlib";

import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchDemoMesh, listDemos } from "./demos";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("fetchDemoMesh", () => {
  it("inflates a still-gzipped response back to the original .glb bytes", async () => {
    const original = new Uint8Array([0x67, 0x6c, 0x54, 0x46, 1, 2, 3, 4, 5]); // 'glTF'…
    const gz = gzipSync(Buffer.from(original));
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response(gz))));

    const out = await fetchDemoMesh("/assets/demos/x.glb.gz");

    expect(Array.from(out)).toEqual(Array.from(original));
  });

  it("passes through bytes the browser already inflated (Content-Encoding: gzip)", async () => {
    // The server set Content-Encoding: gzip, so fetch already handed us glTF.
    // Re-inflating would double-decode — the bug behind "Failed to decode data".
    const glb = new Uint8Array([0x67, 0x6c, 0x54, 0x46, 2, 0, 0, 0]); // 'glTF' header
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response(glb))));

    const out = await fetchDemoMesh("/assets/demos/x.glb.gz");

    expect(Array.from(out)).toEqual(Array.from(glb));
  });

  it("throws on a non-ok response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(new Response(null, { status: 404, statusText: "Not Found" }))),
    );
    await expect(fetchDemoMesh("/nope.glb.gz")).rejects.toThrow(/404/);
  });

  it("throws on an empty response", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response(null))));
    await expect(fetchDemoMesh("/empty.glb.gz")).rejects.toThrow(/empty/);
  });
});

describe("listDemos", () => {
  it("returns a structurally valid list (whatever the pack step produced)", () => {
    const demos = listDemos();
    expect(Array.isArray(demos)).toBe(true);
    for (const d of demos) {
      expect(typeof d.id).toBe("string");
      expect(d.id.length).toBeGreaterThan(0);
      expect(typeof d.title).toBe("string");
      expect(typeof d.url).toBe("string");
      expect(d.url.length).toBeGreaterThan(0);
      expect(typeof d.res).toBe("number");
      expect(typeof d.zUp).toBe("boolean");
      expect(typeof d.truecolor).toBe("boolean");
      expect(typeof d.gpuBake).toBe("boolean");
      expect(["opaque", "mask", "blend"]).toContain(d.alphaMode);
      expect(d.attribution === null || typeof d.attribution === "string").toBe(true);
      expect(d.thumbnail === null || typeof d.thumbnail === "string").toBe(true);
    }
  });

  it("only lists demos whose packed blob resolved to a URL", () => {
    // urlForFile skips index rows without a matching *.glb.gz — so every
    // returned entry has a non-empty url (covered above), and ids are unique.
    const ids = listDemos().map((d) => d.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
