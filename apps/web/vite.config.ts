import { defineConfig } from "vite";
import path from "node:path";
import { fileURLToPath } from "node:url";

const base = process.env.BASE_PATH ?? "/";

export default defineConfig({
  base,
  resolve: {
    alias: [
      {
        find: /^three$/,
        replacement: "three/src/Three.js"
      },
      {
        find: "@gestalt/voxelizer-js",
        replacement: path.resolve(
          fileURLToPath(
            new URL("../../packages/voxelizer-js/src/index.ts", import.meta.url)
          )
        )
      }
    ]
  },
  esbuild: {
    target: "esnext"
  },
  optimizeDeps: {
    esbuildOptions: {
      target: "esnext"
    }
  },
  build: {
    outDir: "dist",
    target: "esnext"
  }
});
