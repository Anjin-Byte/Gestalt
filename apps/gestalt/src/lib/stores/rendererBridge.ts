import { writable } from "svelte/store";
import type { RendererBridge } from "../../renderer/RendererBridge";

/**
 * Holds the active RendererBridge instance.
 * null before initialization or when COOP/COEP headers are absent.
 * Components read this store to send commands; they never import
 * RendererBridge directly.
 */
export const rendererBridgeStore = writable<RendererBridge | null>(null);
