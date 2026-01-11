import type { ModuleOutput, TestbedModule } from "./types";

export const createWasmPointsModule = (): TestbedModule => {
  let wasm: null | {
    default?: () => Promise<unknown>;
    generate_spiral_points?: (
      count: number,
      turns: number,
      radius: number
    ) => { positions?: Float32Array; indices?: Uint32Array };
    init_logging?: () => void;
    log_info?: (message: string) => void;
  } = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;

  const buildSpiralFallback = (
    count: number,
    turns: number,
    radius: number
  ) => {
    const positions = new Float32Array(count * 3);
    for (let i = 0; i < count; i += 1) {
      const t = i / Math.max(1, count - 1);
      const angle = turns * Math.PI * 2 * t;
      const r = radius * t;
      const x = r * Math.cos(angle);
      const y = (t - 0.5) * radius;
      const z = r * Math.sin(angle);
      const base = i * 3;
      positions[base] = x;
      positions[base + 1] = y;
      positions[base + 2] = z;
    }
    return positions;
  };

  const buildRibbonMesh = (positions: Float32Array, thickness: number) => {
    const pointCount = Math.floor(positions.length / 3);
    const ribbonPositions = new Float32Array(pointCount * 2 * 3);
    const normals = new Float32Array(pointCount * 2 * 3);
    const indices: number[] = [];

    const up = { x: 0, y: 1, z: 0 };
    const getPoint = (index: number) => {
      const base = index * 3;
      return {
        x: positions[base],
        y: positions[base + 1],
        z: positions[base + 2]
      };
    };

    for (let i = 0; i < pointCount; i += 1) {
      const current = getPoint(i);
      const next = i < pointCount - 1 ? getPoint(i + 1) : getPoint(i - 1);
      const tangent = {
        x: next.x - current.x,
        y: next.y - current.y,
        z: next.z - current.z
      };
      const tangentLen = Math.hypot(tangent.x, tangent.y, tangent.z) || 1;
      tangent.x /= tangentLen;
      tangent.y /= tangentLen;
      tangent.z /= tangentLen;

      let normal = {
        x: tangent.y * up.z - tangent.z * up.y,
        y: tangent.z * up.x - tangent.x * up.z,
        z: tangent.x * up.y - tangent.y * up.x
      };
      const normalLen = Math.hypot(normal.x, normal.y, normal.z);
      if (normalLen < 1e-5) {
        normal = { x: 1, y: 0, z: 0 };
      } else {
        normal.x /= normalLen;
        normal.y /= normalLen;
        normal.z /= normalLen;
      }

      const offset = {
        x: normal.x * thickness * 0.5,
        y: normal.y * thickness * 0.5,
        z: normal.z * thickness * 0.5
      };

      const leftIndex = i * 2;
      const rightIndex = leftIndex + 1;

      const writeVertex = (vertexIndex: number, x: number, y: number, z: number) => {
        const base = vertexIndex * 3;
        ribbonPositions[base] = x;
        ribbonPositions[base + 1] = y;
        ribbonPositions[base + 2] = z;
        normals[base] = normal.x;
        normals[base + 1] = normal.y;
        normals[base + 2] = normal.z;
      };

      writeVertex(
        leftIndex,
        current.x - offset.x,
        current.y - offset.y,
        current.z - offset.z
      );
      writeVertex(
        rightIndex,
        current.x + offset.x,
        current.y + offset.y,
        current.z + offset.z
      );

      if (i < pointCount - 1) {
        const nextLeft = leftIndex + 2;
        const nextRight = rightIndex + 2;
        indices.push(leftIndex, nextLeft, rightIndex);
        indices.push(rightIndex, nextLeft, nextRight);
      }
    }

    return {
      positions: ribbonPositions,
      normals,
      indices: new Uint32Array(indices)
    };
  };

  return {
    id: "wasm-points",
    name: "WASM Spiral Points",
    init: async (ctx) => {
      try {
        const module = await import("../wasm/wasm_points/wasm_points.js");
        if (module.default) {
          await module.default();
        }
        wasm = module;
        wasm.init_logging?.();
        wasm.log_info?.("WASM points logging initialized.");
        statusText = "Loaded";
        updateStatus?.(statusText);
        ctx.logger.info("WASM points module loaded.");
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(
          `WASM points module failed to load: ${(error as Error).message}`
        );
      }
    },
    ui: (api) => {
      api.addText({ id: "wasm-status", label: "Status", initial: statusText });
      api.addSlider({
        id: "count",
        label: "Point Count",
        min: 32,
        max: 1024,
        step: 32,
        initial: 256
      });
      api.addSlider({
        id: "turns",
        label: "Turns",
        min: 1,
        max: 10,
        step: 1,
        initial: 4
      });
      api.addSlider({
        id: "radius",
        label: "Radius",
        min: 0.5,
        max: 4,
        step: 0.1,
        initial: 2
      });
      api.addSlider({
        id: "thickness",
        label: "Ribbon Thickness",
        min: 0.02,
        max: 0.4,
        step: 0.02,
        initial: 0.12
      });
      updateStatus = (value: string) => api.setText("wasm-status", value);
    },
    run: async (job) => {
      const count = Number(job.params.count ?? 256);
      const turns = Number(job.params.turns ?? 4);
      const radius = Number(job.params.radius ?? 2);
      const thickness = Number(job.params.thickness ?? 0.12);
      wasm?.log_info?.(
        `WASM points run: count=${count} turns=${turns} radius=${radius} thickness=${thickness}`
      );

      let positions: Float32Array;
      if (wasm?.generate_spiral_points) {
        const data = wasm.generate_spiral_points(count, turns, radius);
        if (!data.positions) {
          statusText = "Empty output (build WASM)";
          updateStatus?.(statusText);
          positions = buildSpiralFallback(count, turns, radius);
          statusText = "Fallback (JS)";
        } else if (data.positions.length === 0) {
          statusText = "Empty output (build WASM)";
          updateStatus?.(statusText);
          positions = buildSpiralFallback(count, turns, radius);
          statusText = "Fallback (JS)";
        } else {
          positions = data.positions;
          statusText = "Running";
        }
      } else {
        positions = buildSpiralFallback(count, turns, radius);
        statusText = "Fallback (JS)";
      }

      updateStatus?.(statusText);
      const ribbon = buildRibbonMesh(positions, thickness);
      const output: ModuleOutput = {
        kind: "mesh",
        mesh: {
          positions: ribbon.positions,
          normals: ribbon.normals,
          indices: ribbon.indices
        },
        label: "Spiral Ribbon"
      };

      return [output];
    }
  };
};
