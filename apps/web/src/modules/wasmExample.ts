import type { ModuleOutput, TestbedModule } from "./types";

export const createWasmExampleModule = (): TestbedModule => {
  let wasm: null | {
    default?: () => Promise<unknown>;
    generate_mesh?: () => {
      positions?: Float32Array;
      indices?: Uint32Array;
    };
    init_logging?: () => void;
    log_info?: (message: string) => void;
  } = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;

  return {
    id: "wasm-example",
    name: "WASM Example",
    init: async (ctx) => {
      try {
        const module = await import(
          "../wasm/wasm_example/wasm_example.js"
        );
        if (module.default) {
          await module.default();
        }
        wasm = module;
        wasm.init_logging?.();
        wasm.log_info?.("WASM example logging initialized.");
        statusText = "Loaded";
        updateStatus?.(statusText);
        ctx.logger.info("WASM example module loaded.");
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(
          `WASM example module failed to load. Build it with wasm-pack: ${(error as Error).message}`
        );
      }
    },
    ui: (api) => {
      api.addText({ id: "wasm-status", label: "Status", initial: statusText });
      updateStatus = (value: string) => api.setText("wasm-status", value);
    },
    run: async () => {
      if (!wasm?.generate_mesh) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        return [];
      }

      wasm.log_info?.("WASM example run invoked.");
      const data = wasm.generate_mesh();
      if (!data.positions || !data.indices) {
        statusText = "Empty output (build WASM)";
        updateStatus?.(statusText);
        return [];
      }
      if (data.positions.length === 0) {
        statusText = "Empty output (build WASM)";
        updateStatus?.(statusText);
        return [];
      }
      statusText = "Running";
      updateStatus?.(statusText);
      const output: ModuleOutput = {
        kind: "mesh",
        mesh: {
          positions: data.positions,
          indices: data.indices
        },
        label: "WASM Mesh"
      };

      return [output];
    }
  };
};
