import { expect, test, type Page } from "@playwright/test";

const waitForShell = async (page: Page) => {
  await page.goto("/");
  await page.waitForSelector("body[data-ready='true']");
};

test("loads the app shell", async ({ page }) => {
  await waitForShell(page);
  await expect(page).toHaveTitle(/Gestalt/);
});

test("scene panel is active by default", async ({ page }) => {
  await waitForShell(page);
  // The Scene tab button should be in active state
  await expect(page.locator("button[title='Scene'][data-state='active']")).toBeVisible();
});

test("scene panel shows visualization controls", async ({ page }) => {
  await waitForShell(page);
  // Wireframe, Axes, Grid checkboxes are rendered by CheckboxRow in ScenePanel
  await expect(page.getByText("Wireframe")).toBeVisible();
  await expect(page.getByText("Axes")).toBeVisible();
  await expect(page.getByText("Grid")).toBeVisible();
});

test("clicking debug tab switches to debug panel", async ({ page }) => {
  await waitForShell(page);
  await page.locator("button[title='Debug']").click();
  // DebugPanel renders a "Device" section header
  await expect(page.getByText("Device")).toBeVisible();
  // The Debug tab reflects its active state
  await expect(page.locator("button[title='Debug'][data-state='active']")).toBeVisible();
});

test("clicking performance tab switches to performance panel", async ({ page }) => {
  await waitForShell(page);
  await page.locator("button[title='Performance']").click();
  await expect(page.getByText("Frame Timeline")).toBeVisible();
});

test("clicking settings tab shows settings panel", async ({ page }) => {
  await waitForShell(page);
  await page.locator("button[title='Settings']").click();
  await expect(page.getByText("Renderer")).toBeVisible();
});

test("clicking demo tab renders phi component sections", async ({ page }) => {
  await waitForShell(page);
  await page.locator("button[title='Component Demo']").click();
  // DemoPanel renders Section components for each Phi primitive
  await expect(page.getByText("ScrubField")).toBeVisible();
  await expect(page.getByText("BarMeter")).toBeVisible();
  await expect(page.getByText("StatusIndicator")).toBeVisible();
});

test("panel collapses when active tab is clicked again", async ({ page }) => {
  await waitForShell(page);
  // Click Scene tab (already active) to toggle panel closed
  await page.locator("button[title='Scene']").click();
  // Panel area should collapse — Wireframe label should no longer be visible
  await expect(page.getByText("Wireframe")).not.toBeVisible();
});
