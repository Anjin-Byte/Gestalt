// ─── Splitview: 1D Constraint Solver ────────────────────────────────────────
// Based on dockview's splitview.ts (derived from VS Code).
// Pure TypeScript — no Svelte, no DOM. Operates on abstract views.
// See: reference/dock-implementation-guide.md

// ─── Types ─────────────────────────────────────────────────────────────────

export type Orientation = "horizontal" | "vertical";

export enum LayoutPriority {
  Low = "low",
  Normal = "normal",
  High = "high",
}

export interface IView {
  readonly minimumSize: number;
  readonly maximumSize: number;
  readonly priority?: LayoutPriority;
  readonly snap?: boolean;
  /** Called when the view's size changes. */
  layout(size: number, orthogonalSize: number): void;
  setVisible?(visible: boolean): void;
}

// ─── ViewItem (internal) ───────────────────────────────────────────────────

export class ViewItem {
  private _size: number;
  private _cachedVisibleSize: number | undefined;

  get size(): number { return this._size; }
  set size(s: number) { this._size = s; }

  get visible(): boolean { return this._cachedVisibleSize === undefined; }

  get minimumSize(): number { return this.visible ? this.view.minimumSize : 0; }
  get maximumSize(): number { return this.visible ? this.view.maximumSize : 0; }

  get priority(): LayoutPriority { return this.view.priority ?? LayoutPriority.Normal; }
  get snap(): boolean { return this.view.snap ?? false; }

  constructor(public readonly view: IView, size: number) {
    this._size = size;
    this._cachedVisibleSize = undefined;
  }

  setVisible(visible: boolean, size?: number): void {
    if (visible === this.visible) return;
    if (visible) {
      this._size = clamp(this._cachedVisibleSize ?? 0, this.view.minimumSize, this.view.maximumSize);
      this._cachedVisibleSize = undefined;
    } else {
      this._cachedVisibleSize = typeof size === "number" ? size : this._size;
      this._size = 0;
    }
    this.view.setVisible?.(visible);
  }

  get cachedVisibleSize(): number | undefined { return this._cachedVisibleSize; }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function range(from: number, to: number): number[] {
  const result: number[] = [];
  if (from <= to) {
    for (let i = from; i < to; i++) result.push(i);
  } else {
    for (let i = from; i > to; i--) result.push(i);
  }
  return result;
}

function pushToStart<T>(arr: T[], value: T): void {
  const idx = arr.indexOf(value);
  if (idx > 0) { arr.splice(idx, 1); arr.unshift(value); }
}

function pushToEnd<T>(arr: T[], value: T): void {
  const idx = arr.indexOf(value);
  if (idx >= 0 && idx < arr.length - 1) { arr.splice(idx, 1); arr.push(value); }
}

// ─── Splitview ─────────────────────────────────────────────────────────────

export interface SplitviewOptions {
  orientation: Orientation;
  proportionalLayout?: boolean;
}

export class Splitview {
  private _size = 0;
  private _orthogonalSize = 0;
  private _contentSize = 0;
  private _proportions: (number | undefined)[] | undefined;
  private _items: ViewItem[] = [];

  readonly orientation: Orientation;
  readonly proportionalLayout: boolean;

  get size(): number { return this._size; }
  get items(): readonly ViewItem[] { return this._items; }
  get length(): number { return this._items.length; }

  /** Sum of all item minimumSizes. */
  get minimumSize(): number {
    return this._items.reduce((sum, item) => sum + item.minimumSize, 0);
  }

  /** Sum of all item maximumSizes. */
  get maximumSize(): number {
    return this._items.reduce((sum, item) => sum + item.maximumSize, 0);
  }

  constructor(options: SplitviewOptions) {
    this.orientation = options.orientation;
    this.proportionalLayout = options.proportionalLayout ?? true;
  }

  // ─── Public API ────────────────────────────────────────────────────────

  /** Add a view at the given index with the given initial size. */
  addView(view: IView, size: number, index: number = this._items.length): void {
    const item = new ViewItem(view, size);
    this._items.splice(index, 0, item);
    this._contentSize = this._items.reduce((s, i) => s + i.size, 0);
  }

  /** Remove the view at the given index. Returns the removed view. */
  removeView(index: number): IView {
    const [item] = this._items.splice(index, 1);
    this._contentSize = this._items.reduce((s, i) => s + i.size, 0);
    return item.view;
  }

  /** Get view sizes as an array. */
  getSizes(): number[] {
    return this._items.map((i) => i.size);
  }

  /**
   * Main layout method. Called when the container size changes.
   * Redistributes sizes proportionally (if enabled) or via the constraint solver.
   */
  layout(size: number, orthogonalSize: number): void {
    const previousSize = this._size;
    this._size = size;
    this._orthogonalSize = orthogonalSize;

    if (this._proportions && this.proportionalLayout) {
      // Proportional resize: recompute from saved fractions
      let availableSize = size;
      let totalProportion = 0;

      for (let i = 0; i < this._items.length; i++) {
        const p = this._proportions[i];
        if (typeof p === "number") {
          totalProportion += p;
        } else {
          availableSize -= this._items[i].size; // hidden items keep their size
        }
      }

      for (let i = 0; i < this._items.length; i++) {
        const p = this._proportions[i];
        if (typeof p === "number" && totalProportion > 0) {
          this._items[i].size = clamp(
            Math.round((p * availableSize) / totalProportion),
            this._items[i].minimumSize,
            this._items[i].maximumSize
          );
        }
      }
    } else if (previousSize > 0) {
      // Fixed layout: treat size change as a delta at the last sash
      const delta = size - previousSize;
      if (delta !== 0 && this._items.length > 0) {
        this.resize(this._items.length - 1, delta);
      }
    }

    this.distributeEmptySpace();
    this.layoutViews();
  }

  /**
   * Resize at a sash boundary. Used for interactive sash dragging.
   * @param index Sash index (views[index] is the last view on the "up" side).
   * @param delta Pixel delta (positive = up-group grows, down-group shrinks).
   * @param sizes Optional snapshot of sizes to compute from (for drag-start-relative deltas).
   */
  resize(
    index: number,
    delta: number,
    sizes: number[] = this._items.map((i) => i.size)
  ): number {
    if (index < 0 || index >= this._items.length) return 0;

    // Build index arrays: up = [index, index-1, ..., 0], down = [index+1, ..., n-1]
    const upIndexes = range(index, -1);
    const downIndexes = range(index + 1, this._items.length);

    // Reorder by priority: High first, Low last
    for (const idx of upIndexes.filter((i) => this._items[i].priority === LayoutPriority.High)) {
      pushToStart(upIndexes, idx);
      pushToStart(downIndexes, idx);
    }
    for (const idx of upIndexes.filter((i) => this._items[i].priority === LayoutPriority.Low)) {
      pushToEnd(upIndexes, idx);
      pushToEnd(downIndexes, idx);
    }

    const upSizes = upIndexes.map((i) => sizes[i]);
    const downSizes = downIndexes.map((i) => sizes[i]);

    // Compute feasible delta bounds
    const minDeltaUp = upIndexes.reduce((s, i) => s + this._items[i].minimumSize - sizes[i], 0);
    const maxDeltaUp = upIndexes.reduce((s, i) => s + this._items[i].maximumSize - sizes[i], 0);
    const maxDeltaDown = downIndexes.length === 0
      ? Number.POSITIVE_INFINITY
      : downIndexes.reduce((s, i) => s + sizes[i] - this._items[i].minimumSize, 0);
    const minDeltaDown = downIndexes.length === 0
      ? Number.NEGATIVE_INFINITY
      : downIndexes.reduce((s, i) => s + sizes[i] - this._items[i].maximumSize, 0);

    const minDelta = Math.max(minDeltaUp, minDeltaDown);
    const maxDelta = Math.min(maxDeltaDown, maxDeltaUp);

    const tentativeDelta = clamp(delta, minDelta, maxDelta);

    // Distribute to up-group (greedy walk)
    let actualDelta = 0;
    let deltaUp = tentativeDelta;
    for (let i = 0; i < upIndexes.length; i++) {
      const item = this._items[upIndexes[i]];
      const size = clamp(upSizes[i] + deltaUp, item.minimumSize, item.maximumSize);
      const viewDelta = size - upSizes[i];
      actualDelta += viewDelta;
      deltaUp -= viewDelta;
      item.size = size;
    }

    // Distribute negative to down-group (conservation of space)
    let deltaDown = actualDelta;
    for (let i = 0; i < downIndexes.length; i++) {
      const item = this._items[downIndexes[i]];
      const size = clamp(downSizes[i] - deltaDown, item.minimumSize, item.maximumSize);
      const viewDelta = size - downSizes[i];
      deltaDown += viewDelta;
      item.size = size;
    }

    return tentativeDelta;
  }

  /**
   * After any resize, distribute leftover space (from rounding or clamping).
   * Guarantees sum(sizes) == containerSize.
   */
  distributeEmptySpace(): void {
    const contentSize = this._items.reduce((s, i) => s + i.size, 0);
    let emptyDelta = this._size - contentSize;
    if (emptyDelta === 0) return;

    // Build priority-ordered index list
    const indexes = range(this._items.length - 1, -1);
    for (const i of indexes.filter((i) => this._items[i].priority === LayoutPriority.High)) {
      pushToStart(indexes, i);
    }
    for (const i of indexes.filter((i) => this._items[i].priority === LayoutPriority.Low)) {
      pushToEnd(indexes, i);
    }

    for (let i = 0; emptyDelta !== 0 && i < indexes.length; i++) {
      const item = this._items[indexes[i]];
      const size = clamp(item.size + emptyDelta, item.minimumSize, item.maximumSize);
      const viewDelta = size - item.size;
      emptyDelta -= viewDelta;
      item.size = size;
    }
  }

  /** Save current sizes as proportions for proportional layout. */
  saveProportions(): void {
    if (!this.proportionalLayout) return;
    this._contentSize = this._items.reduce((s, i) => s + i.size, 0);
    if (this._contentSize > 0) {
      this._proportions = this._items.map((i) =>
        i.visible ? i.size / this._contentSize : undefined
      );
    }
  }

  // ─── Internal ──────────────────────────────────────────────────────────

  /** Notify each view of its current size. */
  private layoutViews(): void {
    this._contentSize = this._items.reduce((s, i) => s + i.size, 0);
    for (const item of this._items) {
      item.view.layout(item.size, this._orthogonalSize);
    }
  }
}
