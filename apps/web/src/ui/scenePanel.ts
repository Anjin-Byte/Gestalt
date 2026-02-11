/**
 * Scene panel UI construction.
 *
 * Builds controls for scene visualization: wireframe toggle and framing.
 */

import type { Viewer } from "../viewer/Viewer";

export type ScenePanelOptions = {
  container: HTMLElement;
  viewer: Viewer;
};

/** Build the scene panel UI and attach to the container. */
export function buildScenePanel(options: ScenePanelOptions): void {
  const { container, viewer } = options;

  // Wireframe toggle
  const wireframeToggle = document.createElement("input");
  wireframeToggle.type = "checkbox";
  wireframeToggle.addEventListener("change", () => {
    viewer.setWireframe(wireframeToggle.checked);
  });

  const wireframeLabel = document.createElement("label");
  wireframeLabel.textContent = "Wireframe";
  wireframeLabel.prepend(wireframeToggle);
  container.appendChild(wireframeLabel);

  // Frame button
  const frameButton = document.createElement("button");
  frameButton.textContent = "Frame Object";
  frameButton.addEventListener("click", () => {
    viewer.frameObject();
  });
  container.appendChild(frameButton);
}
