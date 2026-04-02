// ─── TreeList Type Definitions ─────────────────────────────────────────────
// Shared types for the TreeList component system.
// See: reference/gestalt-outliner-spec.md

// ─── Display Items ─────────────────────────────────────────────────────────

/** A flat list of these is the domain adapter's output. */
export type TreeListItem = TreeListGroupItem | TreeListRowItem;

export interface TreeListGroupItem {
  kind: "group";
  /** Stable ID — used as the collapse persistence key. */
  id: string;
  label: string;
  /** Optional aggregate displayed in the group header (e.g. a BarMeter). */
  aggregate?: { value: number; max: number; unit?: string };
  count?: number;
}

export interface TreeListRowItem {
  kind: "row";
  /** Stable ID — used as the selection key. */
  id: string;
  /** Group this row belongs to (determines visibility when group is collapsed). */
  groupId: string;
  /** Indentation depth. 0 = top-level. Each level = 12px left indent. */
  depth?: number;
  /** Lucide icon name, or undefined for no icon. */
  icon?: string;
  label: string;
  /**
   * Secondary icon rendered after the name. Used for status badges that
   * modify the label's meaning (e.g. linked, missing, override).
   */
  statusBadge?: StatusBadge;
  /**
   * At 0.5 opacity — communicates "excluded from active context"
   * (hidden object, excluded collection, disabled modifier, etc.).
   */
  faded?: boolean;
  /** Values for each column slot, in the same order as TreeListDomain.columns. */
  cells: TreeListCellData[];
  draggable?: boolean;
  /** Whether this row's label can be renamed via double-click or F2. Default false. */
  renameable?: boolean;
}

export type StatusBadge =
  | "linked"
  | "linked-indirect"
  | "linked-missing"
  | "override"
  | "override-system"
  | "asset";

// ─── Cell Data ─────────────────────────────────────────────────────────────

import type { Component } from "svelte";

/**
 * Icon for toggle cells. Accepts a Svelte Component (e.g. Lucide icon)
 * or a plain string (rendered as text — fallback for simple cases).
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type CellIcon = Component<any> | string;

export type TreeListCellData =
  | { type: "status"; status: "ok" | "warning" | "error" | "idle"; label?: string }
  | { type: "mono"; value: string }
  | { type: "spark"; values: number[]; warn?: number; danger?: number }
  | { type: "toggle"; value: boolean; icon: CellIcon; disabled?: boolean; propagatable?: boolean };

// ─── Column Definition ─────────────────────────────────────────────────────

export interface TreeListColumnDef {
  /** Unique ID within the domain. Passed to onToggle for toggle cells. */
  id: string;
  /** Fixed width in px. The name column takes all remaining flex space. */
  width: number;
  /**
   * Panel width (in px) below which this column is hidden.
   * Implemented with CSS container queries — no JS needed.
   * Omit for columns that are always visible.
   */
  hideBelow?: number;
  /** Accessible label shown in the column header tooltip. */
  label: string;
}


// ─── Drop Zone ─────────────────────────────────────────────────────────────

export type DropZone = "before" | "into" | "after";

// ─── Context Menu ──────────────────────────────────────────────────────────

import type { ContextMenuItem } from "../ContextMenu.svelte";

// ─── Domain Adapter ────────────────────────────────────────────────────────

export interface TreeListDomain<T> {
  /** Unique domain ID — used to namespace localStorage keys. */
  domainId: string;
  /** Column definitions. Order must match TreeListRowItem.cells order. */
  columns: TreeListColumnDef[];
  /**
   * Maps current domain data to a flat list of display items.
   * Called reactively via $derived — must be a pure function.
   */
  rows(data: T): TreeListItem[];
  /**
   * Called when the user clicks a row. The domain decides what "select" means:
   * focusing a scene object, highlighting a GPU buffer slot, etc.
   */
  onSelect?(id: string): void;
  /**
   * Called when the user clicks a toggle cell.
   * propagate = true when the user held Shift during the click.
   * The domain is responsible for applying propagation to children.
   */
  onToggle?(rowId: string, columnId: string, value: boolean, propagate: boolean): void;
  /**
   * Called when the user drops a dragged row onto a target row.
   * The domain is responsible for the actual mutation and any undo push.
   */
  onDrop?(dragId: string, targetId: string, zone: DropZone): void;
  /**
   * Called when the user confirms an inline rename (double-click or F2, then Enter).
   * The domain validates and applies the rename. Return false to reject.
   */
  onRename?(id: string, newLabel: string): void;
  /**
   * Called when the user presses Delete/Backspace on the selected row(s).
   * The domain decides whether to show confirmation or delete immediately.
   */
  onDelete?(ids: string[]): void;
  /**
   * Called when the user presses Ctrl+D / Cmd+D on the selected row(s).
   * The domain creates duplicates and returns the new IDs (for auto-selection).
   */
  onDuplicate?(ids: string[]): void;
  /**
   * Returns context menu items for a given row. Called on right-click.
   * Return an empty array to suppress the menu. Omit to disable context menus entirely.
   */
  getContextItems?(id: string): ContextMenuItem[];
  /**
   * Called when a context menu item is selected.
   * The domain executes the action. Standard actions (rename, delete, duplicate)
   * can reuse the existing callbacks or implement custom ones.
   */
  onContextAction?(rowId: string, actionId: string): void;
}

// ─── TreeList State Store ──────────────────────────────────────────────────

/**
 * Wraps localStorage for collapse state persistence. One instance per domain,
 * keyed by domainId. Follows the same pattern as Section.svelte's localStorage.
 */
export class TreeListStateStore {
  private key: string;
  private collapsed: Set<string>;

  constructor(domainId: string) {
    this.key = `treelist:${domainId}:collapsed`;
    const saved = localStorage.getItem(this.key);
    this.collapsed = new Set(saved ? JSON.parse(saved) : []);
  }

  isCollapsed(groupId: string): boolean {
    return this.collapsed.has(groupId);
  }

  toggle(groupId: string): void {
    if (this.collapsed.has(groupId)) {
      this.collapsed.delete(groupId);
    } else {
      this.collapsed.add(groupId);
    }
    this.save();
  }

  expand(groupId: string): void {
    this.collapsed.delete(groupId);
    this.save();
  }

  collapse(groupId: string): void {
    this.collapsed.add(groupId);
    this.save();
  }

  /** Force-expand all given group IDs (used by filter and show-active). */
  expandAll(groupIds: string[]): void {
    for (const id of groupIds) {
      this.collapsed.delete(id);
    }
    this.save();
  }

  get collapsedSet(): ReadonlySet<string> {
    return this.collapsed;
  }

  private save(): void {
    localStorage.setItem(this.key, JSON.stringify([...this.collapsed]));
  }
}
