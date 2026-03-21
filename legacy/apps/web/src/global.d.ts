interface Navigator {
  gpu?: GPU;
}

interface GPUDevice extends EventTarget {}

interface GPUUncapturedErrorEvent extends Event {
  error: { message: string };
}

interface GPUQueue {
  writeBuffer: (...args: unknown[]) => void;
  submit: (...args: unknown[]) => void;
}

interface GPUDevice {
  createShaderModule: (...args: unknown[]) => GPUShaderModule;
  createComputePipeline: (...args: unknown[]) => GPUComputePipeline;
  createBuffer: (...args: unknown[]) => GPUBuffer;
  createBindGroup: (...args: unknown[]) => GPUBindGroup;
  createCommandEncoder: (...args: unknown[]) => GPUCommandEncoder;
  queue: GPUQueue;
  limits: {
    maxComputeInvocationsPerWorkgroup: number;
    maxStorageBufferBindingSize: number;
    maxStorageBuffersPerShaderStage: number;
    maxComputeWorkgroupsPerDimension: number;
  };
}

interface GPUAdapter {
  requestDevice: () => Promise<GPUDevice>;
}

interface GPUCommandEncoder {
  beginComputePass: (...args: unknown[]) => GPUComputePassEncoder;
  copyBufferToBuffer: (...args: unknown[]) => void;
  finish: () => unknown;
}

interface GPUComputePassEncoder {
  setPipeline: (...args: unknown[]) => void;
  setBindGroup: (...args: unknown[]) => void;
  dispatchWorkgroups: (...args: unknown[]) => void;
  end: () => void;
}

interface GPUBuffer {
  mapAsync: (...args: unknown[]) => Promise<void>;
  getMappedRange: () => ArrayBuffer;
  unmap: () => void;
}

interface GPUBindGroup {}
interface GPUShaderModule {}

declare const GPUBufferUsage: {
  STORAGE: number;
  COPY_SRC: number;
  COPY_DST: number;
  UNIFORM: number;
  MAP_READ: number;
};

declare const GPUMapMode: {
  READ: number;
};

type GPUComputePipeline = {
  getBindGroupLayout: (index: number) => GPUBindGroupLayout;
};
type GPUBindGroupLayout = unknown;
