/**
 * Patch GPUAdapter.requestDevice to strip deprecated WebGPU limits
 * that Firefox rejects.
 *
 * wgpu 22's WASM backend unconditionally includes `maxInterStageShaderComponents`
 * in the requiredLimits object passed to requestDevice(). Firefox removed this
 * limit from the spec and throws an OperationError if it's present. This patch
 * intercepts the call and deletes the offending property before forwarding.
 *
 * Safe to call on browsers that don't have WebGPU — it no-ops.
 */
export function patchWebGpuLimits(): void {
  if (typeof globalThis.GPUAdapter === "undefined") return;

  const original = GPUAdapter.prototype.requestDevice;
  GPUAdapter.prototype.requestDevice = function (
    descriptor?: GPUDeviceDescriptor,
  ): Promise<GPUDevice> {
    if (descriptor?.requiredLimits) {
      // Remove limits that Firefox doesn't recognize
      delete (descriptor.requiredLimits as Record<string, unknown>)[
        "maxInterStageShaderComponents"
      ];
    }
    return original.call(this, descriptor);
  };
}
