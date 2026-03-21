import type { Logger } from "../modules/types";

export type WebgpuLoggerHandle = {
  detach: () => void;
  hasDevice: boolean;
};

export const attachWebgpuErrorLogger = (
  renderer: unknown,
  logger: Logger
): WebgpuLoggerHandle => {
  const backend = (renderer as { backend?: { device?: GPUDevice } }).backend;
  const device = backend?.device;

  if (!device || !device.addEventListener) {
    return {
      detach: () => undefined,
      hasDevice: false
    };
  }

  const handler: EventListener = (event) => {
    const gpuEvent = event as GPUUncapturedErrorEvent;
    logger.error(`WebGPU error: ${gpuEvent.error.message}`);
  };

  device.addEventListener("uncapturederror", handler);

  return {
    detach: () => {
      device.removeEventListener("uncapturederror", handler);
    },
    hasDevice: true
  };
};
