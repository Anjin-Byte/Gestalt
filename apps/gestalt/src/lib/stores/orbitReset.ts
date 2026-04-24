import { writable } from "svelte/store";

export interface OrbitResetData {
  center: [number, number, number];
  extent: number;
}

/** Set to trigger orbit camera reset after model load. */
export const orbitReset = writable<OrbitResetData | null>(null);
