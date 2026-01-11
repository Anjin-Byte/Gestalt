import type { ModuleOutput, TestbedModule } from "./types";

type WasmWebgpu = {
  default?: () => Promise<unknown>;
  wgsl_source?: () => string;
  init_logging?: () => void;
  log_info?: (message: string) => void;
};

const buildCircleFallback = (count: number, radius: number) => {
  const positions = new Float32Array(count * 3);
  for (let i = 0; i < count; i += 1) {
    const t = count <= 1 ? 0 : i / (count - 1);
    const angle = t * Math.PI * 2;
    const base = i * 3;
    positions[base] = Math.cos(angle) * radius;
    positions[base + 1] = Math.sin(angle) * radius;
    positions[base + 2] = 0;
  }
  return positions;
};

export const createWasmWebgpuDemoModule = (): TestbedModule => {
  let wasm: WasmWebgpu | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;
  let device: GPUDevice | null = null;
  let pipeline: GPUComputePipeline | null = null;
  let bindGroupLayout: GPUBindGroupLayout | null = null;
  let ctxRef: { requestGpuDevice: () => Promise<GPUDevice | null> } | null = null;

  const ensurePipeline = async (ctx: { requestGpuDevice: () => Promise<GPUDevice | null> }) => {
    if (pipeline && device) {
      return;
    }
    device = await ctx.requestGpuDevice();
    if (!device) {
      return;
    }
    const source = wasm?.wgsl_source?.() ?? "";
    if (!source) {
      return;
    }
    const module = device.createShaderModule({ code: source });
    pipeline = device.createComputePipeline({
      layout: "auto",
      compute: { module, entryPoint: "main" }
    });
    bindGroupLayout = pipeline.getBindGroupLayout(0);
  };

  const runWebgpu = async (
    ctx: { requestGpuDevice: () => Promise<GPUDevice | null> },
    count: number,
    radius: number
  ) => {
    await ensurePipeline(ctx);
    if (!device || !pipeline || !bindGroupLayout) {
      return null;
    }

    const bufferSize = count * 4 * 4;
    const storageBuffer = device.createBuffer({
      size: bufferSize,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
    });

    const uniformBuffer = device.createBuffer({
      size: 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST
    });

    const params = new ArrayBuffer(16);
    const paramsView = new DataView(params);
    paramsView.setUint32(0, count, true);
    paramsView.setFloat32(4, radius, true);
    device.queue.writeBuffer(uniformBuffer, 0, params);

    const bindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: storageBuffer } },
        { binding: 1, resource: { buffer: uniformBuffer } }
      ]
    });

    const commandEncoder = device.createCommandEncoder();
    const pass = commandEncoder.beginComputePass();
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.dispatchWorkgroups(Math.ceil(count / 64));
    pass.end();

    const readBuffer = device.createBuffer({
      size: bufferSize,
      usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ
    });
    commandEncoder.copyBufferToBuffer(storageBuffer, 0, readBuffer, 0, bufferSize);
    device.queue.submit([commandEncoder.finish()]);

    await readBuffer.mapAsync(GPUMapMode.READ);
    const mapped = readBuffer.getMappedRange();
    const data = new Float32Array(mapped.slice(0));
    readBuffer.unmap();

    const positions = new Float32Array(count * 3);
    for (let i = 0; i < count; i += 1) {
      const src = i * 4;
      const dst = i * 3;
      positions[dst] = data[src];
      positions[dst + 1] = data[src + 1];
      positions[dst + 2] = data[src + 2];
    }

    return positions;
  };

  return {
    id: "wasm-webgpu-demo",
    name: "WASM WebGPU Demo",
    init: async (ctx) => {
      ctxRef = ctx;
      try {
        const module = await import("../wasm/wasm_webgpu_demo/wasm_webgpu_demo.js");
        if (module.default) {
          await module.default();
        }
        wasm = module as unknown as WasmWebgpu;
        wasm.init_logging?.();
        wasm.log_info?.("WASM WebGPU demo logging initialized.");
        statusText = "Loaded";
        updateStatus?.(statusText);
        ctx.logger.info("WASM WebGPU demo module loaded.");
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`WASM WebGPU demo failed to load: ${(error as Error).message}`);
      }
    },
    ui: (api) => {
      api.addText({ id: "wasm-status", label: "Status", initial: statusText });
      api.addSlider({
        id: "count",
        label: "Point Count",
        min: 64,
        max: 2048,
        step: 64,
        initial: 512
      });
      api.addSlider({
        id: "radius",
        label: "Radius",
        min: 0.5,
        max: 5,
        step: 0.1,
        initial: 2
      });
      updateStatus = (value: string) => api.setText("wasm-status", value);
    },
    run: async (job) => {
      const count = Number(job.params.count ?? 512);
      const radius = Number(job.params.radius ?? 2);
      wasm?.log_info?.(`WASM WebGPU demo run: count=${count} radius=${radius}`);

      let positions: Float32Array | null = null;
      let hadWgsl = false;
      if (wasm?.wgsl_source && ctxRef) {
        const wgsl = wasm.wgsl_source();
        hadWgsl = wgsl.length > 0;
        positions = hadWgsl ? await runWebgpu(ctxRef, count, radius) : null;
      }

      if (!positions || positions.length === 0) {
        if (!device) {
          statusText = "WebGPU unavailable";
        } else if (!hadWgsl) {
          statusText = "Missing (run pnpm build:wasm)";
        } else {
          statusText = "Fallback (JS)";
        }
        updateStatus?.(statusText);
        positions = buildCircleFallback(count, radius);
      } else {
        statusText = "WebGPU";
        updateStatus?.(statusText);
      }

      const output: ModuleOutput = {
        kind: "points",
        points: {
          positions,
          color: [0.2, 0.9, 0.5],
          size: 0.06
        },
        label: "GPU Circle"
      };

      return [output];
    }
  };
};
