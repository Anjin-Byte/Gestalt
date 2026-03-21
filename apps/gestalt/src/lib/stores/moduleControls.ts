import { writable, get } from "svelte/store";
import type { UiApi, UiControl } from "@gestalt/modules";

export const moduleControls = writable<UiControl[]>([]);
export const moduleValues = writable<Record<string, unknown>>({});

/** Set by App.svelte once the module host is ready. */
export const scheduleRun = writable<() => void>(() => {});

/** Creates a UiApi impl that writes to the Svelte stores above. */
export function createSvelteUiApi(): UiApi {
  return {
    addSlider: ({ id, label, min, max, step, initial }) => {
      moduleValues.update(v => ({ ...v, [id]: initial }));
      moduleControls.update(cs => [...cs, { kind: "slider", id, label, min, max, step, initial }]);
    },
    addNumber: ({ id, label, min, max, step, initial }) => {
      moduleValues.update(v => ({ ...v, [id]: initial }));
      moduleControls.update(cs => [...cs, { kind: "number", id, label, min, max, step, initial }]);
    },
    addCheckbox: ({ id, label, initial }) => {
      moduleValues.update(v => ({ ...v, [id]: initial }));
      moduleControls.update(cs => [...cs, { kind: "checkbox", id, label, initial }]);
    },
    addSelect: ({ id, label, options, initial }) => {
      moduleValues.update(v => ({ ...v, [id]: initial }));
      moduleControls.update(cs => [...cs, { kind: "select", id, label, options, initial }]);
    },
    addText: ({ id, label, initial }) => {
      moduleControls.update(cs => [...cs, { kind: "text", id, label, initial }]);
    },
    setText: (id, value) => {
      moduleControls.update(cs =>
        cs.map(c => (c.kind === "text" && c.id === id ? { ...c, initial: value } : c))
      );
    },
    addFile: ({ id, label, accept, onFile }) => {
      moduleControls.update(cs => [...cs, { kind: "file", id, label, accept, onFile }]);
    },
    addButton: ({ label, onClick }) => {
      moduleControls.update(cs => [...cs, { kind: "button", label, onClick }]);
    },
    getValues: () => get(moduleValues),
    clear: () => {
      moduleControls.set([]);
      moduleValues.set({});
    },
  };
}
