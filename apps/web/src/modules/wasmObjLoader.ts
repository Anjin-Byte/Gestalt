import type { ModuleOutput, TestbedModule } from "./types";

type WasmObjLoader = {
  default?: () => Promise<unknown>;
  parse_obj?: (input: string) => { positions: Float32Array; indices: Uint32Array };
  wgsl_source?: () => string;
  transform_matrix?: (
    scale: number,
    rotX: number,
    rotY: number,
    rotZ: number,
    tx: number,
    ty: number,
    tz: number
  ) => Float32Array;
  init_logging?: () => void;
  log_info?: (message: string) => void;
};

const parseObjFallback = (input: string) => {
  const positions: number[] = [];
  const indices: number[] = [];

  const lines = input.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("v ")) {
      const parts = trimmed.split(/\s+/);
      if (parts.length >= 4) {
        const x = Number(parts[1]);
        const y = Number(parts[2]);
        const z = Number(parts[3]);
        if (Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z)) {
          positions.push(x, y, z);
        }
      }
    } else if (trimmed.startsWith("f ")) {
      const parts = trimmed.split(/\s+/).slice(1);
      const faceIndices = parts
        .map((part) => Number(part.split("/")[0]))
        .filter((value) => Number.isFinite(value) && value > 0)
        .map((value) => value - 1);

      if (faceIndices.length >= 3) {
        const base = faceIndices[0];
        for (let i = 1; i < faceIndices.length - 1; i += 1) {
          indices.push(base, faceIndices[i], faceIndices[i + 1]);
        }
      }
    }
  }

  return {
    positions: new Float32Array(positions),
    indices: new Uint32Array(indices)
  };
};

export const createWasmObjLoaderModule = (): TestbedModule => {
  let wasm: WasmObjLoader | null = null;
  let statusText = "Not loaded";
  let fileName = "No file";
  let objText = "";
  let updateStatus: ((value: string) => void) | null = null;
  let updateFileName: ((value: string) => void) | null = null;
  let logger: ((message: string) => void) | null = null;
  let ctxRef: { requestGpuDevice: () => Promise<GPUDevice | null> } | null = null;
  let device: GPUDevice | null = null;
  let pipeline: GPUComputePipeline | null = null;
  let bindGroupLayout: GPUBindGroupLayout | null = null;

  const buildMatrixFallback = (
    scale: number,
    rotX: number,
    rotY: number,
    rotZ: number,
    tx: number,
    ty: number,
    tz: number
  ) => {
    const toRad = (deg: number) => (deg * Math.PI) / 180;
    const [sx, cx] = [Math.sin(toRad(rotX)), Math.cos(toRad(rotX))];
    const [sy, cy] = [Math.sin(toRad(rotY)), Math.cos(toRad(rotY))];
    const [sz, cz] = [Math.sin(toRad(rotZ)), Math.cos(toRad(rotZ))];

    const s = [
      scale, 0, 0, 0,
      0, scale, 0, 0,
      0, 0, scale, 0,
      0, 0, 0, 1
    ];
    const rx = [
      1, 0, 0, 0,
      0, cx, sx, 0,
      0, -sx, cx, 0,
      0, 0, 0, 1
    ];
    const ry = [
      cy, 0, -sy, 0,
      0, 1, 0, 0,
      sy, 0, cy, 0,
      0, 0, 0, 1
    ];
    const rz = [
      cz, sz, 0, 0,
      -sz, cz, 0, 0,
      0, 0, 1, 0,
      0, 0, 0, 1
    ];
    const t = [
      1, 0, 0, 0,
      0, 1, 0, 0,
      0, 0, 1, 0,
      tx, ty, tz, 1
    ];

    const mul = (a: number[], b: number[]) => {
      const out = new Array<number>(16).fill(0);
      for (let c = 0; c < 4; c += 1) {
        for (let r = 0; r < 4; r += 1) {
          let sum = 0;
          for (let k = 0; k < 4; k += 1) {
            sum += a[k * 4 + r] * b[c * 4 + k];
          }
          out[c * 4 + r] = sum;
        }
      }
      return out;
    };

    const rs = mul(rz, mul(ry, mul(rx, s)));
    const m = mul(t, rs);
    return new Float32Array(m);
  };

  const applyMatrix = (positions: Float32Array, matrix: Float32Array) => {
    const count = Math.floor(positions.length / 3);
    const out = new Float32Array(count * 3);
    for (let i = 0; i < count; i += 1) {
      const src = i * 3;
      const x = positions[src];
      const y = positions[src + 1];
      const z = positions[src + 2];
      const ox =
        matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12];
      const oy =
        matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13];
      const oz =
        matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14];
      out[src] = ox;
      out[src + 1] = oy;
      out[src + 2] = oz;
    }
    return out;
  };

  const ensurePipeline = async () => {
    if (pipeline && device) {
      return;
    }
    if (!ctxRef) {
      return;
    }
    device = await ctxRef.requestGpuDevice();
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

  const runWebgpuTransform = async (
    positions: Float32Array,
    matrix: Float32Array
  ) => {
    try {
      await ensurePipeline();
      if (!device || !pipeline || !bindGroupLayout) {
        return null;
      }

    const count = Math.floor(positions.length / 3);
    const input = new Float32Array(count * 4);
    for (let i = 0; i < count; i += 1) {
      const src = i * 3;
      const dst = i * 4;
      input[dst] = positions[src];
      input[dst + 1] = positions[src + 1];
      input[dst + 2] = positions[src + 2];
      input[dst + 3] = 1;
    }

    const bufferSize = input.byteLength;
    const inputBuffer = device.createBuffer({
      size: bufferSize,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST
    });
    device.queue.writeBuffer(inputBuffer, 0, input);

    const outputBuffer = device.createBuffer({
      size: bufferSize,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC
    });

    const matrixBuffer = device.createBuffer({
      size: 64,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST
    });
    device.queue.writeBuffer(matrixBuffer, 0, matrix);

    const params = new ArrayBuffer(16);
    const paramsView = new DataView(params);
    paramsView.setUint32(0, count, true);
    const paramsBuffer = device.createBuffer({
      size: 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST
    });
    device.queue.writeBuffer(paramsBuffer, 0, params);

    const bindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: inputBuffer } },
        { binding: 1, resource: { buffer: outputBuffer } },
        { binding: 2, resource: { buffer: matrixBuffer } },
        { binding: 3, resource: { buffer: paramsBuffer } }
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
    commandEncoder.copyBufferToBuffer(outputBuffer, 0, readBuffer, 0, bufferSize);
      device.queue.submit([commandEncoder.finish()]);

      await readBuffer.mapAsync(GPUMapMode.READ);
      const mapped = readBuffer.getMappedRange();
      const data = new Float32Array(mapped.slice(0));
      readBuffer.unmap();

      const transformed = new Float32Array(count * 3);
      for (let i = 0; i < count; i += 1) {
        const src = i * 4;
        const dst = i * 3;
        transformed[dst] = data[src];
        transformed[dst + 1] = data[src + 1];
        transformed[dst + 2] = data[src + 2];
      }

      return transformed;
    } catch (error) {
      logger?.(`OBJ loader: WebGPU transform failed: ${(error as Error).message}`);
      return null;
    }
  };

  return {
    id: "wasm-obj-loader",
    name: "WASM OBJ Loader",
    init: async (ctx) => {
      ctxRef = ctx;
      logger = ctx.logger.info;
      try {
        const module = await import("../wasm/wasm_obj_loader/wasm_obj_loader.js");
        if (module.default) {
          await module.default();
        }
        wasm = module as unknown as WasmObjLoader;
        wasm.init_logging?.();
        wasm.log_info?.("WASM OBJ loader logging initialized.");
        statusText = "Loaded";
        updateStatus?.(statusText);
        ctx.logger.info("WASM OBJ loader module loaded.");
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(
          `WASM OBJ loader failed to load: ${(error as Error).message}`
        );
      }
    },
    ui: (api) => {
      api.addText({ id: "wasm-status", label: "Status", initial: statusText });
      api.addText({ id: "obj-file", label: "OBJ File", initial: fileName });
      api.addFile({
        id: "obj-input",
        label: "Pick OBJ",
        accept: ".obj",
        onFile: async (file) => {
          if (!file) {
            fileName = "No file";
            objText = "";
            statusText = "No file selected";
            updateFileName?.(fileName);
            updateStatus?.(statusText);
            logger?.("OBJ loader: no file selected.");
            return;
          }
          fileName = file.name;
          statusText = "Loading...";
          updateFileName?.(fileName);
          updateStatus?.(statusText);
          logger?.(`OBJ loader: reading ${file.name} (${file.size} bytes).`);
          try {
            objText = await file.text();
            statusText = `Ready (${objText.length} chars)`;
            updateStatus?.(statusText);
            logger?.(`OBJ loader: file loaded (${objText.length} chars).`);
          } catch (error) {
            statusText = "File read failed";
            updateStatus?.(statusText);
            logger?.(`OBJ loader: file read failed: ${(error as Error).message}`);
          }
        }
      });
      api.addSlider({
        id: "scale",
        label: "Scale",
        min: 0.1,
        max: 5,
        step: 0.1,
        initial: 1
      });
      api.addSlider({
        id: "rot-x",
        label: "Rotate X (deg)",
        min: -180,
        max: 180,
        step: 1,
        initial: 0
      });
      api.addSlider({
        id: "rot-y",
        label: "Rotate Y (deg)",
        min: -180,
        max: 180,
        step: 1,
        initial: 0
      });
      api.addSlider({
        id: "rot-z",
        label: "Rotate Z (deg)",
        min: -180,
        max: 180,
        step: 1,
        initial: 0
      });
      api.addSlider({
        id: "tx",
        label: "Translate X",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0
      });
      api.addSlider({
        id: "ty",
        label: "Translate Y",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0
      });
      api.addSlider({
        id: "tz",
        label: "Translate Z",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0
      });
      updateStatus = (value: string) => api.setText("wasm-status", value);
      updateFileName = (value: string) => api.setText("obj-file", value);
    },
    run: async (job) => {
      if (!objText) {
        statusText = "No OBJ loaded";
        updateStatus?.(statusText);
        logger?.("OBJ loader: run skipped (no OBJ loaded).");
        return [];
      }

      const scale = Number(job.params.scale ?? 1);
      const rotX = Number(job.params["rot-x"] ?? 0);
      const rotY = Number(job.params["rot-y"] ?? 0);
      const rotZ = Number(job.params["rot-z"] ?? 0);
      const tx = Number(job.params.tx ?? 0);
      const ty = Number(job.params.ty ?? 0);
      const tz = Number(job.params.tz ?? 0);
      wasm?.log_info?.(
        `WASM OBJ loader run: scale=${scale} rot=(${rotX},${rotY},${rotZ}) ` +
          `t=(${tx},${ty},${tz}) chars=${objText.length}`
      );
      let data;
      if (wasm?.parse_obj) {
        data = wasm.parse_obj(objText);
        if (data.positions.length === 0) {
          statusText = "Empty output (build WASM)";
          updateStatus?.(statusText);
          logger?.("OBJ loader: WASM returned empty output, falling back to JS.");
          data = parseObjFallback(objText);
          statusText = "Fallback (JS)";
        } else {
          statusText = "Running";
        }
      } else {
        data = parseObjFallback(objText);
        statusText = "Fallback (JS)";
        logger?.("OBJ loader: WASM unavailable, using JS fallback.");
      }

      updateStatus?.(statusText);

      if (data.positions.length === 0 || data.indices.length === 0) {
        statusText = "No faces found";
        updateStatus?.(statusText);
        logger?.(
          `OBJ loader: no faces found (positions=${data.positions.length}, indices=${data.indices.length}).`
        );
        return [];
      }

      let finalPositions: Float32Array | null = null;
      const matrix =
        wasm?.transform_matrix?.(scale, rotX, rotY, rotZ, tx, ty, tz) ??
        buildMatrixFallback(scale, rotX, rotY, rotZ, tx, ty, tz);
      if (wasm?.wgsl_source && ctxRef) {
        const wgsl = wasm.wgsl_source();
        if (wgsl.length > 0) {
          finalPositions = await runWebgpuTransform(
            data.positions,
            matrix
          );
          if (!finalPositions) {
            logger?.("OBJ loader: WebGPU transform unavailable, using CPU path.");
          }
        }
      }

      if (!finalPositions) {
        finalPositions = applyMatrix(data.positions, matrix);
        statusText = device ? "Fallback (CPU)" : "WebGPU unavailable";
      } else {
        statusText = "Transformed (WebGPU)";
      }
      updateStatus?.(statusText);

      logger?.(
        `OBJ loader: parsed positions=${data.positions.length} indices=${data.indices.length}.`
      );

      const output: ModuleOutput = {
        kind: "mesh",
        mesh: {
          positions: finalPositions,
          indices: data.indices
        },
        label: fileName
      };

      return [output];
    }
  };
};
