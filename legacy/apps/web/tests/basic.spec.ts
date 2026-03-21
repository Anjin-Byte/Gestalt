import { expect, test, type Page } from "@playwright/test";

const gotoTestMode = async (page: Page) => {
  await page.goto("/?test=1");
  await page.waitForSelector("body[data-ready='true']");
};

test("loads home page", async ({ page }) => {
  await gotoTestMode(page);
  await expect(page).toHaveTitle(/Gestalt Test Bed/);
  await expect(page.locator("#viewport")).toBeVisible();
});

test("renders test mesh", async ({ page }) => {
  await gotoTestMode(page);
  await page.locator("[data-testid='module-select']").selectOption("hello-triangle");
  await page.locator("[data-testid='run-module']").click();
  await expect(page.locator("[data-testid='overlay']")).toContainText("Triangles");
});

test("renders voxel debug output", async ({ page }) => {
  await gotoTestMode(page);
  await page.locator("[data-testid='module-select']").selectOption("voxels-debug");
  await page.locator("[data-testid='run-module']").click();
  await expect(page.locator("[data-testid='overlay']")).toContainText("Instances");
});

test("screenshot regression", async ({ page }) => {
  await gotoTestMode(page);
  await page.locator("[data-testid='module-select']").selectOption("hello-triangle");
  await page.locator("[data-testid='run-module']").click();
  await page.waitForTimeout(200);
  await expect(page).toHaveScreenshot("gestalt-home.png");
});
