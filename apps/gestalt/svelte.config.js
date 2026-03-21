import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
export default {
  // vitePreprocess uses Vite's transform pipeline for TypeScript and
  // style preprocessing inside .svelte files. Required for svelte-check
  // to correctly resolve path aliases and handle <script lang="ts">.
  preprocess: [vitePreprocess()],
};
