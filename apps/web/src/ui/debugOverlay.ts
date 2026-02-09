/**
 * Debug overlay for displaying structured performance and memory statistics.
 *
 * Provides a HUD-style overlay that floats over the viewport, independent of
 * module parameters. Modules can push stats to named sections.
 */

export type DebugSection = "memory" | "performance" | "chunks" | "custom";

export interface DebugOverlayOptions {
  /** Container element to attach the overlay to. */
  container: HTMLElement;
  /** Initial visibility state. */
  visible?: boolean;
}

interface SectionConfig {
  label: string;
  order: number;
}

const SECTION_CONFIG: Record<DebugSection, SectionConfig> = {
  memory: { label: "Memory", order: 0 },
  performance: { label: "Performance", order: 1 },
  chunks: { label: "Chunks", order: 2 },
  custom: { label: "Debug", order: 3 },
};

/**
 * Debug overlay manager.
 *
 * Usage:
 * ```ts
 * const overlay = new DebugOverlay({ container: viewportEl });
 * overlay.update("memory", [
 *   { label: "Voxel", value: "2.1 MB" },
 *   { label: "Mesh", value: "0.8 MB" },
 * ]);
 * ```
 */
export class DebugOverlay {
  private root: HTMLElement;
  private sections: Map<DebugSection, HTMLElement> = new Map();
  private sectionData: Map<DebugSection, Array<{ label: string; value: string }>> = new Map();
  private visible: boolean;

  constructor(options: DebugOverlayOptions) {
    this.visible = options.visible ?? true;

    // Create root overlay element
    this.root = document.createElement("div");
    this.root.className = "debug-overlay";
    this.root.style.display = this.visible ? "flex" : "none";

    options.container.appendChild(this.root);
  }

  /**
   * Update a section with new stat entries.
   *
   * @param section - Section identifier
   * @param entries - Array of label/value pairs to display
   */
  update(section: DebugSection, entries: Array<{ label: string; value: string }>): void {
    this.sectionData.set(section, entries);
    this.render();
  }

  /**
   * Clear a specific section.
   */
  clear(section: DebugSection): void {
    this.sectionData.delete(section);
    this.render();
  }

  /**
   * Clear all sections.
   */
  clearAll(): void {
    this.sectionData.clear();
    this.render();
  }

  /**
   * Toggle overlay visibility.
   */
  toggle(): void {
    this.visible = !this.visible;
    this.root.style.display = this.visible ? "flex" : "none";
  }

  /**
   * Set overlay visibility.
   */
  setVisible(visible: boolean): void {
    this.visible = visible;
    this.root.style.display = this.visible ? "flex" : "none";
  }

  /**
   * Check if overlay is visible.
   */
  isVisible(): boolean {
    return this.visible;
  }

  /**
   * Dispose of the overlay and remove from DOM.
   */
  dispose(): void {
    this.root.remove();
    this.sections.clear();
    this.sectionData.clear();
  }

  private render(): void {
    // Clear existing content
    this.root.innerHTML = "";
    this.sections.clear();

    // Sort sections by order
    const sortedSections = Array.from(this.sectionData.entries()).sort(
      ([a], [b]) => SECTION_CONFIG[a].order - SECTION_CONFIG[b].order
    );

    // Render each section
    for (const [sectionId, entries] of sortedSections) {
      if (entries.length === 0) continue;

      const config = SECTION_CONFIG[sectionId];
      const sectionEl = this.createSection(config.label, entries);
      this.sections.set(sectionId, sectionEl);
      this.root.appendChild(sectionEl);
    }
  }

  private createSection(
    label: string,
    entries: Array<{ label: string; value: string }>
  ): HTMLElement {
    const section = document.createElement("div");
    section.className = "debug-section";

    // Section header
    const header = document.createElement("div");
    header.className = "debug-section-header";
    header.textContent = label;
    section.appendChild(header);

    // Section content - grid of label/value pairs
    const content = document.createElement("div");
    content.className = "debug-section-content";

    for (const entry of entries) {
      const row = document.createElement("div");
      row.className = "debug-row";

      const labelEl = document.createElement("span");
      labelEl.className = "debug-label";
      labelEl.textContent = entry.label;

      const valueEl = document.createElement("span");
      valueEl.className = "debug-value";
      valueEl.textContent = entry.value;

      row.appendChild(labelEl);
      row.appendChild(valueEl);
      content.appendChild(row);
    }

    section.appendChild(content);
    return section;
  }
}

// Singleton instance for global access
let globalOverlay: DebugOverlay | null = null;

/**
 * Initialize the global debug overlay.
 * Call this once during app initialization.
 */
export function initDebugOverlay(options: DebugOverlayOptions): DebugOverlay {
  if (globalOverlay) {
    globalOverlay.dispose();
  }
  globalOverlay = new DebugOverlay(options);
  return globalOverlay;
}

/**
 * Get the global debug overlay instance.
 * Returns null if not initialized.
 */
export function getDebugOverlay(): DebugOverlay | null {
  return globalOverlay;
}

// Convenience helpers for common stat formatting
export const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
};

export const formatMs = (ms: number): string => `${ms.toFixed(1)}ms`;

export const formatPercent = (ratio: number): string => `${(ratio * 100).toFixed(1)}%`;

export const formatCount = (count: number): string => {
  if (count < 1000) return String(count);
  if (count < 1_000_000) return `${(count / 1000).toFixed(1)}K`;
  return `${(count / 1_000_000).toFixed(2)}M`;
};
