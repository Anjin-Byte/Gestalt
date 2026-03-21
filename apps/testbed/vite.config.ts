import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [
    svelte(),
    tailwindcss(),
  ],
  resolve: {
    alias: [
      {
        find: /^three$/,
        replacement: "three/src/Three.js",
      },
      {
        find: "@gestalt/modules",
        replacement: path.resolve(
          fileURLToPath(
            new URL("../../packages/modules/src/index.ts", import.meta.url)
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
        find: "@web",
        replacement: path.resolve(
          fileURLToPath(
            new URL("../../apps/web/src", import.meta.url)
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
  },
});
