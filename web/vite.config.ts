import { defineConfig } from "vite";

export default defineConfig({
  // The wasm-pack package arrives via the `file:` dependency (a symlink into
  // crates/voxel-web/pkg). esbuild pre-bundling would inline the bindgen glue
  // and break its `import.meta.url`-relative wasm resolution, so it is excluded
  // from optimization and served/bundled as-is.
  optimizeDeps: { exclude: ["voxel-web"] },
  // The symlink's real path is outside web/, so the dev server must be allowed
  // to serve from the repo root.
  server: { fs: { allow: [".."] } },
  build: { target: "es2022", sourcemap: true },
});
