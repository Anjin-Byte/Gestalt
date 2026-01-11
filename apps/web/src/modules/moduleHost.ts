import type { ModuleContext, ModuleOutput, TestbedModule } from "./types";
import { createUiApi } from "../ui/uiApi";

export class ModuleHost {
  private modules: TestbedModule[];
  private active: TestbedModule | null = null;
  private frameId = 0;
  private uiApi;
  private runInProgress = false;
  private runQueued = false;
  private lastParamsKey = "";
  private pendingParamsKey: string | null = null;

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
      await module.init(this.ctx);
    }
  }

  list(): TestbedModule[] {
    return [...this.modules];
  }

  async activate(moduleId: string): Promise<void> {
    const next = this.modules.find((module) => module.id === moduleId) ?? null;
    if (!next || next === this.active) {
      return;
    }

    this.uiApi.clear();
    this.active?.dispose?.();
    this.active = next;
    this.onOutputs([]);
    this.ctx.logger.info(`Activated module ${next.id}`);

    if (this.active.ui) {
      this.active.ui(this.uiApi);
    }

    this.scheduleRun();
  }

  private async scheduleRun(): Promise<void> {
    const paramsKey = JSON.stringify(this.uiApi.getValues());
    if (!this.runInProgress && paramsKey === this.lastParamsKey) {
      return;
    }
    this.pendingParamsKey = paramsKey;
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
      this.pendingParamsKey = null;
      this.scheduleRun();
    }
  }

  async runActive(): Promise<void> {
    if (!this.active) {
      return;
    }

    try {
      const params = this.uiApi.getValues();
      const paramsKey = JSON.stringify(params);
      this.lastParamsKey = paramsKey;
      const outputs = await this.active.run({
        params,
        frameId: this.frameId++
      });
      this.onOutputs(outputs);
    } catch (error) {
      this.ctx.logger.error(
        `Module ${this.active.id} run exception: ${(error as Error).message}`
      );
      this.onOutputs([]);
    }
  }
}
