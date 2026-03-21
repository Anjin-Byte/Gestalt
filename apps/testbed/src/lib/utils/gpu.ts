let gpuDevicePromise: Promise<GPUDevice | null> | null = null;

export const requestGpuDevice = (): Promise<GPUDevice | null> => {
  if (!("gpu" in navigator)) return Promise.resolve(null);
  if (!gpuDevicePromise) {
    gpuDevicePromise = navigator.gpu
      .requestAdapter()
      .then((a: GPUAdapter | null) => (a ? a.requestDevice() : null));
  }
  return gpuDevicePromise;
};
