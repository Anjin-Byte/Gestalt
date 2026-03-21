import { writable } from "svelte/store";
import type { Viewer } from "@web/viewer/Viewer";
import type { ViewerBackend } from "@web/viewer/threeBackend";

export const viewerStore = writable<Viewer | null>(null);
export const backendStore = writable<ViewerBackend | null>(null);
export const fpsText = writable<string>("");
