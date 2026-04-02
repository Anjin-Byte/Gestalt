// ─── Gridview: 2D Layout as a Recursive Tree of Splitviews ─────────────────
// Based on dockview's gridview.ts (derived from VS Code).
// Pure TypeScript — no DOM, no Svelte. The tree model + operations.
// See: reference/dock-implementation-guide.md

import { Splitview, type IView, type Orientation, LayoutPriority } from "./Splitview";

// ─── Types ─────────────────────────────────────────────────────────────────

export type Direction = "up" | "down" | "left" | "right";

export function orthogonal(orientation: Orientation): Orientation {
  return orientation === "horizontal" ? "vertical" : "horizontal";
}

function directionOrientation(direction: Direction): Orientation {
  return direction === "left" || direction === "right" ? "horizontal" : "vertical";
}

/** A panel that can live in the grid. Uses absolute width/height (not orientation-relative). */
export interface IGridView {
  readonly minimumWidth: number;
  readonly maximumWidth: number;
  readonly minimumHeight: number;
  readonly maximumHeight: number;
  readonly priority?: LayoutPriority;
  layout(width: number, height: number): void;
}

// ─── Node Types ────────────────────────────────────────────────────────────

export type GridNode = BranchNode | LeafNode;

export class LeafNode {
  readonly kind = "leaf" as const;
  size = 0;
  orthogonalSize = 0;

  get minimumSize(): number {
    return this.orientation === "horizontal" ? this.view.minimumHeight : this.view.minimumWidth;
  }
  get maximumSize(): number {
    return this.orientation === "horizontal" ? this.view.maximumHeight : this.view.maximumWidth;
  }
  get minimumOrthogonalSize(): number {
    return this.orientation === "horizontal" ? this.view.minimumWidth : this.view.minimumHeight;
  }
  get maximumOrthogonalSize(): number {
    return this.orientation === "horizontal" ? this.view.maximumWidth : this.view.maximumHeight;
  }

  get width(): number {
    return this.orientation === "horizontal" ? this.orthogonalSize : this.size;
  }
  get height(): number {
    return this.orientation === "horizontal" ? this.size : this.orthogonalSize;
  }

  constructor(
    public readonly view: IGridView,
    public readonly orientation: Orientation,
  ) {}

  layout(size: number, orthogonalSize: number): void {
    this.size = size;
    this.orthogonalSize = orthogonalSize;
    this.view.layout(this.width, this.height);
  }
}

export class BranchNode {
  readonly kind = "branch" as const;
  readonly splitview: Splitview;
  readonly children: GridNode[] = [];
  size = 0;
  orthogonalSize = 0;

  /** Min size along the split axis = sum of children's min sizes. */
  get minimumOrthogonalSize(): number {
    return this.children.reduce((s, c) => s + c.minimumSize, 0);
  }
  /** Max size along the split axis = sum of children's max sizes. */
  get maximumOrthogonalSize(): number {
    return this.children.reduce((s, c) => s + c.maximumSize, 0);
  }
  /** Min size perpendicular to split = max of children's min orthogonal sizes. */
  get minimumSize(): number {
    if (this.children.length === 0) return 0;
    return Math.max(...this.children.map((c) => c.minimumOrthogonalSize));
  }

  get maximumSize(): number {
    if (this.children.length === 0) return Number.POSITIVE_INFINITY;
    return Math.min(...this.children.map((c) => c.maximumOrthogonalSize));
  }

  constructor(
    public readonly orientation: Orientation,
  ) {
    this.splitview = new Splitview({ orientation, proportionalLayout: true });
  }

  /** Add a child node at the given index with the given size. */
  addChild(node: GridNode, size: number, index: number = this.children.length): void {
    this.children.splice(index, 0, node);
    // Create an IView adapter for the splitview
    const viewAdapter: IView = {
      minimumSize: node.minimumSize,
      maximumSize: node.maximumSize,
      priority: node.kind === "leaf" ? node.view.priority : undefined,
      layout: (s, o) => node.layout(s, o),
    };
    this.splitview.addView(viewAdapter, size, index);
  }

  /** Remove the child at the given index. Returns [removedNode, itsSize]. */
  removeChild(index: number): [GridNode, number] {
    const size = this.splitview.getSizes()[index];
    this.splitview.removeView(index);
    const [node] = this.children.splice(index, 1);
    return [node, size];
  }

  layout(size: number, orthogonalSize: number): void {
    this.size = size;
    this.orthogonalSize = orthogonalSize;
    this.splitview.layout(orthogonalSize, size);
  }

  /** Resize a sash between children. */
  resizeChild(index: number, delta: number, sizes?: number[]): void {
    this.splitview.resize(index, delta, sizes);
  }
}

// ─── Gridview ──────────────────────────────────────────────────────────────

export class Gridview {
  private _root: BranchNode;
  private _width = 0;
  private _height = 0;

  get root(): BranchNode { return this._root; }
  get orientation(): Orientation { return this._root.orientation; }
  get width(): number { return this._width; }
  get height(): number { return this._height; }

  constructor(orientation: Orientation = "horizontal") {
    this._root = new BranchNode(orientation);
  }

  // ─── Layout ──────────────────────────────────────────────────────────

  layout(width: number, height: number): void {
    this._width = width;
    this._height = height;
    this._root.layout(
      this.orientation === "horizontal" ? height : width,
      this.orientation === "horizontal" ? width : height,
    );
  }

  // ─── Add View ────────────────────────────────────────────────────────

  /**
   * Add a view at a tree location.
   * @param location Path of child indices from root. E.g., [0, 1] = root.children[0].children[1].
   * @param view The grid view to add.
   * @param size Initial size in pixels.
   */
  addView(view: IGridView, size: number, location: number[]): void {
    const [rest, index] = tail(location);
    const [pathToParent, parent] = this.getNode(rest);

    if (parent.kind === "branch") {
      // Simple case: insert into an existing branch
      const leaf = new LeafNode(view, orthogonal(parent.orientation));
      parent.addChild(leaf, size, index);
    } else {
      // Complex case: the target is a leaf — need to restructure
      // Replace the leaf with a new branch containing the old leaf + new view
      const grandparent = pathToParent.length > 0
        ? pathToParent[pathToParent.length - 1]
        : this._root;
      const leafIndex = rest[rest.length - 1];

      if (grandparent.kind !== "branch") return;

      const [oldLeaf, oldSize] = grandparent.removeChild(leafIndex);
      if (oldLeaf.kind !== "leaf") return;

      // Create new branch with orientation opposite to grandparent
      const newBranch = new BranchNode(orthogonal(grandparent.orientation));

      // Re-wrap old leaf with the correct orientation for the new branch
      const rewrappedOld = new LeafNode(oldLeaf.view, orthogonal(newBranch.orientation));
      const newLeaf = new LeafNode(view, orthogonal(newBranch.orientation));

      // Distribute size: old leaf gets remaining space, new leaf gets requested size
      const oldLeafSize = Math.max(0, oldSize - size);

      if (index === 0) {
        newBranch.addChild(newLeaf, size, 0);
        newBranch.addChild(rewrappedOld, oldLeafSize, 1);
      } else {
        newBranch.addChild(rewrappedOld, oldLeafSize, 0);
        newBranch.addChild(newLeaf, size, 1);
      }

      grandparent.addChild(newBranch, oldSize, leafIndex);
    }
  }

  /**
   * Add a view relative to another view using a direction.
   */
  addViewAt(view: IGridView, size: number, direction: Direction, targetLocation: number[]): void {
    const relativeLocation = this.getRelativeLocation(direction, targetLocation);
    this.addView(view, size, relativeLocation);
  }

  // ─── Remove View ─────────────────────────────────────────────────────

  /**
   * Remove the view at the given location.
   * Collapses single-child branches to maintain tree balance.
   */
  removeView(location: number[]): IGridView {
    const [rest, index] = tail(location);
    const [pathToParent, parent] = this.getNode(rest);

    if (parent.kind !== "branch") throw new Error("Cannot remove from a leaf");

    const [removed] = parent.removeChild(index);
    if (removed.kind !== "leaf") throw new Error("Can only remove leaf nodes");

    // Collapse single-child branches
    if (parent.children.length === 1 && pathToParent.length > 0) {
      const grandparent = pathToParent[pathToParent.length - 1];
      if (grandparent.kind !== "branch") return removed.view;

      const parentIndex = rest[rest.length - 1];
      const [, parentSize] = grandparent.removeChild(parentIndex);
      const sibling = parent.children[0];

      if (sibling.kind === "leaf") {
        // Promote the sibling leaf into the grandparent
        const promoted = new LeafNode(sibling.view, orthogonal(grandparent.orientation));
        grandparent.addChild(promoted, parentSize, parentIndex);
      } else {
        // Splice the sibling branch's children into the grandparent
        const sibSizes = sibling.splitview.getSizes();
        for (let i = 0; i < sibling.children.length; i++) {
          const child = sibling.children[i];
          if (child.kind === "leaf") {
            const promoted = new LeafNode(child.view, orthogonal(grandparent.orientation));
            grandparent.addChild(promoted, sibSizes[i], parentIndex + i);
          } else {
            grandparent.addChild(child, sibSizes[i], parentIndex + i);
          }
        }
      }
    } else if (parent === this._root && parent.children.length === 1) {
      const sibling = parent.children[0];
      if (sibling.kind === "branch") {
        // Promote sibling branch to root
        this._root = sibling;
      }
    }

    return removed.view;
  }

  // ─── Serialization ───────────────────────────────────────────────────

  serialize(): SerializedGridview {
    return {
      root: serializeNode(this._root),
      width: this._width,
      height: this._height,
      orientation: this.orientation,
    };
  }

  static deserialize(data: SerializedGridview, viewFactory: (id: string) => IGridView): Gridview {
    const grid = new Gridview(data.orientation);
    grid._root = deserializeBranch(data.root as SerializedBranch, data.orientation, viewFactory);
    grid._width = data.width;
    grid._height = data.height;
    return grid;
  }

  // ─── Internals ───────────────────────────────────────────────────────

  /**
   * Walk the tree to find the node at a location.
   * Returns [path of BranchNodes visited, target node].
   */
  private getNode(location: number[]): [BranchNode[], GridNode] {
    let node: GridNode = this._root;
    const path: BranchNode[] = [];

    for (const index of location) {
      if (node.kind !== "branch") throw new Error(`Expected branch at path, got leaf`);
      path.push(node);
      node = node.children[index];
      if (!node) throw new Error(`Invalid location: no child at index ${index}`);
    }

    return [path, node];
  }

  /**
   * Convert a direction + target location into a tree location for insertion.
   * This is the key algorithm that makes 2D operations work on a 1D tree.
   */
  private getRelativeLocation(direction: Direction, targetLocation: number[]): number[] {
    const dirOrientation = directionOrientation(direction);
    const targetDepth = targetLocation.length;

    // Determine what orientation the target level has
    let levelOrientation = this.orientation;
    for (let i = 0; i < targetDepth - 1; i++) {
      levelOrientation = orthogonal(levelOrientation);
    }

    // If the direction matches the target level's orientation:
    // → insert as a sibling (adjust last index)
    if (dirOrientation === levelOrientation) {
      const location = [...targetLocation];
      const lastIndex = location[location.length - 1];
      location[location.length - 1] = direction === "right" || direction === "down"
        ? lastIndex + 1
        : lastIndex;
      return location;
    }

    // If the direction is orthogonal to the target level:
    // → push one level deeper (triggers tree restructuring in addView)
    const location = [...targetLocation];
    location.push(direction === "right" || direction === "down" ? 1 : 0);
    return location;
  }
}

// ─── Serialization Types ───────────────────────────────────────────────────

export interface SerializedGridview {
  root: SerializedNode;
  width: number;
  height: number;
  orientation: Orientation;
}

export type SerializedNode = SerializedBranch | SerializedLeaf;

export interface SerializedBranch {
  type: "branch";
  size: number;
  data: SerializedNode[];
}

export interface SerializedLeaf {
  type: "leaf";
  size: number;
  data: string; // panel/view ID
}

function serializeNode(node: GridNode): SerializedNode {
  if (node.kind === "leaf") {
    return { type: "leaf", size: node.size, data: (node.view as any).id ?? "" };
  }
  const sizes = node.splitview.getSizes();
  return {
    type: "branch",
    size: node.size,
    data: node.children.map((child, i) => {
      const serialized = serializeNode(child);
      serialized.size = sizes[i];
      return serialized;
    }),
  };
}

function deserializeBranch(
  data: SerializedBranch,
  orientation: Orientation,
  viewFactory: (id: string) => IGridView,
): BranchNode {
  const branch = new BranchNode(orientation);
  const childOrientation = orthogonal(orientation);

  for (const childData of data.data) {
    if (childData.type === "leaf") {
      const view = viewFactory(childData.data);
      const leaf = new LeafNode(view, childOrientation);
      branch.addChild(leaf, childData.size);
    } else {
      const childBranch = deserializeBranch(childData, childOrientation, viewFactory);
      branch.addChild(childBranch, childData.size);
    }
  }

  return branch;
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/** Split an array into [init, last]. E.g., [1,2,3] → [[1,2], 3]. */
function tail(arr: number[]): [number[], number] {
  if (arr.length === 0) throw new Error("Cannot tail empty array");
  return [arr.slice(0, -1), arr[arr.length - 1]];
}
