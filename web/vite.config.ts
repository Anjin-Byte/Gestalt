import { defineConfig } from "vite";

// GitHub Pages serves this project at https://<user>.github.io/Gestalt/,
// so a production build must be based under that sub-path or every hashed asset
// URL (JS, wasm, workers, packed demos) 404s. Dev and preview stay at root; the
// Pages workflow builds with `command === "build"`.
export default defineConfig(({ command }) => ({
  base: command === "build" ? "/Gestalt/" : "/",
  // The wasm-pack package arrives via the `file:` dependency (a symlink into
  // crates/voxel-web/pkg). esbuild pre-bundling would inline the bindgen glue
  // and break its `import.meta.url`-relative wasm resolution, so it is excluded
  // from optimization and served/bundled as-is.
  optimizeDeps: { exclude: ["voxel-web"] },
  // The symlink's real path is outside web/, so the dev server must be allowed
  // to serve from the repo root.
  server: { fs: { allow: [".."] } },
  build: { target: "es2022", sourcemap: true },
}));
