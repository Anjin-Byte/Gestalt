import type { TestbedModule } from "@gestalt/modules";

/** Stub module — exercises the full UI control pipeline without WASM. */
export const stubModule: TestbedModule = {
  id: "stub",
  name: "Stub",
  init: async () => {},
  ui: (api) => {
    api.addSlider({ id: "scale", label: "Scale", min: 0.1, max: 10, step: 0.1, initial: 1 });
    api.addCheckbox({ id: "visible", label: "Visible", initial: true });
    api.addButton({ label: "Log Values", onClick: () => console.log("[stub] values logged") });
  },
  run: async () => [],
};
