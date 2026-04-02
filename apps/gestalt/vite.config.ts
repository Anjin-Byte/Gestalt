import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const _require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------
// Cross-Origin Isolation headers
// Required for SharedArrayBuffer (renderer worker state readback ring buffer).
// See: docs/architecture/wasm-boundary-protocol.md
// ---------------------------------------------------------------------------
const COOP_COEP_HEADERS = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

// ---------------------------------------------------------------------------
// coi-serviceworker plugin
// Injects the coi-serviceworker.js file so that GitHub Pages (which cannot
// serve custom HTTP headers) gets COOP/COEP injected via service worker.
// The dev server sets the headers directly; this plugin handles production.
// ---------------------------------------------------------------------------
function coiServiceWorkerPlugin(): Plugin {
  const coiSrc = (): string => {
    const resolved = _require.resolve("coi-serviceworker/coi-serviceworker.js");
    return fs.readFileSync(resolved, "utf8");
  };

  return {
    name: "coi-serviceworker",
    apply: "build",
    generateBundle() {
      this.emitFile({
        type: "asset",
        fileName: "coi-serviceworker.js",
        source: coiSrc(),
      });
    },
  };
}

export default defineConfig({
  base: process.env.BASE_PATH || "/",

  plugins: [
    svelte(),
    tailwindcss(),
    coiServiceWorkerPlugin(),
  ],

  server: {
    headers: COOP_COEP_HEADERS,
  },

  preview: {
    headers: COOP_COEP_HEADERS,
  },

  resolve: {
    alias: [
      {
        find: /^three$/,
        replacement: "three/src/Three.js",
      },
      {
        find: "@gestalt/phi",
        replacement: path.resolve(
          fileURLToPath(
            new URL("../../packages/phi/src/index.ts", import.meta.url)
          )
        ),
      },
      {
        find: "@gestalt/voxelizer-js",
        replacement: path.resolve(
          fileURLToPath(
            new URL("../../packages/voxelizer-js/src/index.ts", import.meta.url)
          )
        ),
      },
      {
        find: "$lib",
        replacement: path.resolve(
          fileURLToPath(new URL("./src/lib", import.meta.url))
        ),
      },
    ],
  },

  esbuild: {
    target: "esnext",
  },

  optimizeDeps: {
    esbuildOptions: {
      target: "esnext",
    },
  },

  worker: {
    format: "es",
  },

  build: {
    outDir: "dist",
    target: "esnext",
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("wasm_renderer")) return "wasm-renderer";
        },
      },
    },
  },
});
