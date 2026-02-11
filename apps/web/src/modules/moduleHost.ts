import type { ModuleContext, ModuleOutput, TestbedModule } from "./types";
import { createUiApi } from "../ui/uiApi";

export class ModuleHost {
  private modules: TestbedModule[];
  private active: TestbedModule | null = null;
  private frameId = 0;
  private uiApi;
  private initializedModuleIds = new Set<string>();
  private runInProgress = false;
  private runQueued = false;
  private lastParamsKey = "";
  private activeRunController: AbortController | null = null;
  private activationVersion = 0;

  constructor(
    modules: TestbedModule[],
    uiContainer: HTMLElement,
    private readonly ctx: ModuleContext,
    private readonly onOutputs: (outputs: ModuleOutput[]) => void
  ) {
    this.modules = modules;
    this.ctx.emitOutputs = onOutputs;
    this.uiApi = createUiApi(uiContainer, () => {
      this.scheduleRun();
    });
  }

  async initAll(): Promise<void> {
    for (const module of this.modules) {
      await this.ensureInitialized(module, this.activationVersion);
    }
  }

  list(): TestbedModule[] {
    return [...this.modules];
  }

  async activate(moduleId: string): Promise<void> {
    const activationVersion = ++this.activationVersion;
    const next = this.modules.find((module) => module.id === moduleId) ?? null;
    if (!next || next === this.active) {
      return;
    }

    this.cancelActiveRun();
    const previous = this.active;
    if (previous) {
      try {
        await previous.deactivate?.();
      } catch (error) {
        this.ctx.logger.warn(
          `Module ${previous.id} deactivate failed: ${(error as Error).message}`
        );
      }
    }

    if (activationVersion !== this.activationVersion) {
      return;
    }

    this.uiApi.clear();
    this.active = null;
    this.lastParamsKey = "";
    this.runQueued = false;
    this.onOutputs([]);

    await this.ensureInitialized(next, activationVersion);
    if (activationVersion !== this.activationVersion) {
      return;
    }

    try {
      await next.activate?.(this.ctx);
    } catch (error) {
      this.ctx.logger.warn(
        `Module ${next.id} activate failed: ${(error as Error).message}`
      );
    }

    if (activationVersion !== this.activationVersion) {
      return;
    }

    this.active = next;
    this.ctx.logger.info(`Activated module ${next.id}`);

    if (this.active.ui) {
      this.active.ui(this.uiApi);
    }

    this.scheduleRun();
  }

  private async scheduleRun(): Promise<void> {
    if (!this.active) {
      return;
    }
    const paramsKey = JSON.stringify(this.uiApi.getValues());
    if (!this.runInProgress && paramsKey === this.lastParamsKey) {
      return;
    }
    if (this.runInProgress) {
      this.runQueued = true;
      return;
    }

    this.runInProgress = true;
    this.ctx.logger.info("Running active module...");
    try {
      await this.runActive();
    } catch (error) {
      this.ctx.logger.error(`Module run failed: ${(error as Error).message}`);
    } finally {
      this.ctx.logger.info("Active module run complete.");
      this.runInProgress = false;
    }

    if (this.runQueued) {
      this.runQueued = false;
      this.scheduleRun();
    }
  }

  async runActive(): Promise<void> {
    if (!this.active) {
      return;
    }

    const moduleAtStart = this.active;
    const activationVersion = this.activationVersion;
    const runController = new AbortController();
    this.activeRunController = runController;

    try {
      const params = this.uiApi.getValues();
      const paramsKey = JSON.stringify(params);
      this.lastParamsKey = paramsKey;
      const outputs = await moduleAtStart.run({
        params,
        frameId: this.frameId++,
        signal: runController.signal,
        moduleId: moduleAtStart.id
      });
      const isStale =
        runController.signal.aborted ||
        activationVersion !== this.activationVersion ||
        this.active !== moduleAtStart;
      if (!isStale) {
        this.onOutputs(outputs);
      }
    } catch (error) {
      const message = (error as Error).message;
      const isAbort =
        runController.signal.aborted ||
        (error instanceof DOMException && error.name === "AbortError");
      if (isAbort) {
        return;
      }
      this.ctx.logger.error(
        `Module ${moduleAtStart.id} run exception: ${message}`
      );
      if (activationVersion === this.activationVersion && this.active === moduleAtStart) {
        this.onOutputs([]);
      }
    } finally {
      if (this.activeRunController === runController) {
        this.activeRunController = null;
      }
    }
  }

  async dispose(): Promise<void> {
    this.cancelActiveRun();
    const modules = [...this.modules];
    for (const module of modules) {
      try {
        await module.deactivate?.();
      } catch (error) {
        this.ctx.logger.warn(
          `Module ${module.id} deactivate failed during dispose: ${(error as Error).message}`
        );
      }
      try {
        module.dispose?.();
      } catch (error) {
        this.ctx.logger.warn(
          `Module ${module.id} dispose failed: ${(error as Error).message}`
        );
      }
    }
    this.active = null;
    this.uiApi.clear();
    this.onOutputs([]);
  }

  private async ensureInitialized(module: TestbedModule, activationVersion: number): Promise<void> {
    if (this.initializedModuleIds.has(module.id)) {
      return;
    }
    await module.init(this.ctx);
    if (activationVersion !== this.activationVersion) {
      return;
    }
    this.initializedModuleIds.add(module.id);
  }

  private cancelActiveRun(): void {
    this.activeRunController?.abort();
    this.activeRunController = null;
    this.runQueued = false;
  }
}
