/**
 * Dock DnD — drag-and-drop utilities for panel rearrangement.
 *
 * Zone detection, payload types, and direction mapping for the
 * 5-zone drop target system (center + 4 edges).
 *
 * See: reference/dock-implementation-guide.md — DnD Implementation Details
 */

import type { Direction } from "./Gridview";

// ─── Types ──────────────────────────────────────────────────────────────

/** Drop zone position within a dock group's bounding rect. */
export type DropZonePosition = "center" | "top" | "right" | "bottom" | "left";

/** Data transferred during a tab drag operation. */
export interface PanelDragPayload {
  panelId: string;
  sourceGroupId: string;
}

/** MIME type for panel drag data in dataTransfer. */
export const PANEL_DRAG_MIME = "application/x-phi-panel";

// ─── Zone Detection ─────────────────────────────────────────────────────

/**
 * Detect which drop zone the cursor is in relative to a bounding rect.
 *
 * Uses a 20% edge threshold (matching dockview). The cursor must be within
 * the outer 20% of an edge to trigger that edge zone; otherwise it's "center".
 *
 * Edge priority: left/right checked before top/bottom (horizontal layout bias).
 * This means corners favor horizontal splits, matching the behavior of
 * horizontal-first dock layouts.
 *
 * @param rect - The target element's bounding client rect
 * @param x - Cursor clientX
 * @param y - Cursor clientY
 * @param edgeThreshold - Fraction of dimension for edge zones (default 0.2 = 20%)
 */
export function detectZone(
  rect: DOMRect,
  x: number,
  y: number,
  edgeThreshold = 0.2,
): DropZonePosition {
  const relX = (x - rect.left) / rect.width;
  const relY = (y - rect.top) / rect.height;

  // Check horizontal edges first (left/right priority for horizontal-first layouts)
  if (relX < edgeThreshold) return "left";
  if (relX > 1 - edgeThreshold) return "right";
  if (relY < edgeThreshold) return "top";
  if (relY > 1 - edgeThreshold) return "bottom";
  return "center";
}

// ─── Direction Mapping ──────────────────────────────────────────────────

/**
 * Map a drop zone position to a Gridview direction for addViewAt().
 * "center" has no direction — it means tabify (add to existing group).
 */
export function zoneToDirection(zone: DropZonePosition): Direction | null {
  switch (zone) {
    case "left": return "left";
    case "right": return "right";
    case "top": return "up";
    case "bottom": return "down";
    case "center": return null;
  }
}

// ─── Payload Helpers ────────────────────────────────────────────────────

/** Encode a panel drag payload into a drag event's dataTransfer. */
export function setDragPayload(dt: DataTransfer, payload: PanelDragPayload): void {
  dt.setData(PANEL_DRAG_MIME, JSON.stringify(payload));
  dt.effectAllowed = "move";
}

/** Decode a panel drag payload from a drag event's dataTransfer. Returns null if not a panel drag. */
export function getDragPayload(dt: DataTransfer): PanelDragPayload | null {
  const raw = dt.getData(PANEL_DRAG_MIME);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as PanelDragPayload;
  } catch {
    return null;
  }
}

/** Check if a drag event contains panel drag data (without reading it — useful for dragover). */
export function hasPanelDrag(dt: DataTransfer): boolean {
  return dt.types.includes(PANEL_DRAG_MIME);
}
