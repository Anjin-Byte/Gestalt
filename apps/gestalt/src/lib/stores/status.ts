import { writable } from "svelte/store";

export const statusHint = writable<string>("");

export function setHint(text: string): void {
  statusHint.set(text);
}

export function clearHint(): void {
  statusHint.set("");
}
