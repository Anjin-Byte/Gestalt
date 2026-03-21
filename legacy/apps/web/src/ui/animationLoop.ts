/**
 * Animation loop with frame rate limiting and FPS tracking.
 *
 * Manages the render loop, frame timing, and status bar updates.
 */

export type AnimationLoopOptions = {
  /** Called each frame to render. */
  render: () => void;
  /** Element to display FPS stats. */
  statusElement: HTMLElement;
};

/**
 * Animation loop controller.
 *
 * Handles frame rate limiting, FPS calculation, and render scheduling.
 */
export class AnimationLoop {
  private render: () => void;
  private statusElement: HTMLElement;

  private targetFps = 0;
  private lastFrameTime = 0;
  private lastSampleTime = 0;
  private frameCount = 0;
  private running = false;
  private animationId = 0;

  constructor(options: AnimationLoopOptions) {
    this.render = options.render;
    this.statusElement = options.statusElement;
  }

  /** Start the animation loop. */
  start(): void {
    if (this.running) return;
    this.running = true;
    this.lastFrameTime = performance.now();
    this.lastSampleTime = performance.now();
    this.frameCount = 0;
    this.tick();
  }

  /** Stop the animation loop. */
  stop(): void {
    this.running = false;
    if (this.animationId) {
      cancelAnimationFrame(this.animationId);
      this.animationId = 0;
    }
  }

  /** Set target frame rate (0 = uncapped). */
  setTargetFps(fps: number): void {
    this.targetFps = fps;
    this.lastFrameTime = performance.now();
  }

  private tick = (): void => {
    if (!this.running) return;

    const now = performance.now();

    // Frame rate limiting
    if (this.targetFps > 0) {
      const frameDuration = 1000 / this.targetFps;
      if (now - this.lastFrameTime < frameDuration) {
        this.animationId = requestAnimationFrame(this.tick);
        return;
      }
      this.lastFrameTime = now;
    }

    // Render
    this.render();

    // Update FPS stats
    this.updateStatus(now);

    // Schedule next frame
    this.animationId = requestAnimationFrame(this.tick);
  };

  private updateStatus(now: number): void {
    this.frameCount += 1;
    const delta = now - this.lastSampleTime;

    if (delta >= 500) {
      const fps = Math.round((this.frameCount / delta) * 1000);
      const frameTime = (delta / this.frameCount).toFixed(1);
      this.statusElement.textContent = `FPS: ${fps} | Frame: ${frameTime} ms`;
      this.lastSampleTime = now;
      this.frameCount = 0;
    }
  }
}
