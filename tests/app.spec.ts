import { expect, test } from "@playwright/test";

test("creates a group scene, tags it, edits a node, comments, copies, and exports markdown", async ({ page }, testInfo) => {
  await page.goto("/");

  await expect(page.locator(".canvas-watermark").getByText("shape.ai")).toBeVisible();
  await expect(page.getByRole("button", { name: "Zoom out" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Zoom in" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Fit scene" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Fullscreen" })).toBeVisible();
  await page.getByRole("button", { name: "Open groups" }).click();
  await expect(page.getByRole("button", { name: "Close groups" })).toBeVisible();

  const tagName = `Review ${testInfo.project.name} ${Date.now()}`;
  await page.getByLabel("New tag name").fill(tagName);
  await page.getByRole("button", { name: "Create tag" }).click();
  await expect(page.locator(".sidebar .tag-chip").filter({ hasText: tagName }).first()).toBeVisible();
  await page.locator(".sidebar .tag-chip").filter({ hasText: tagName }).first().click();
  await expect(page.locator(".sidebar .tag-chip.is-active").filter({ hasText: tagName }).first()).toBeVisible();

  await page.getByRole("button", { name: "Create group" }).click();
  await expect(page.locator(".decision-node").first()).toBeVisible();
  await expect(page.locator(".group-tag-panel")).toHaveCount(0);
  await expect(page.locator(".scene-group-frame").filter({ hasText: tagName }).first()).toBeVisible();
  await page.getByRole("button", { name: "Open groups" }).click();
  await expect(page.locator(".selected-group-tags .tag-chip.is-attached").filter({ hasText: tagName }).first()).toBeVisible();
  await page.getByRole("button", { name: "Close groups" }).click();

  const firstNode = page.locator(".decision-node").first();
  await expect(firstNode.getByLabel("Node title")).toHaveCount(0);

  await firstNode.click();
  await expect(firstNode).toHaveClass(/is-selected/);
  await expect(firstNode.locator(".node-note-scroll")).toBeVisible();
  await expect(firstNode.getByRole("button", { name: "Edit node" })).toBeVisible();
  await expect(firstNode.getByLabel("Node title")).toHaveCount(0);
  await expect
    .poll(async () => firstNode.locator(".node-note-scroll").evaluate((element) => getComputedStyle(element).overflowY))
    .toBe("auto");

  await page.keyboard.press("e");
  await expect(firstNode.getByLabel("Node title")).toBeVisible();
  await expect(firstNode.getByLabel("Type")).toBeVisible();

  await firstNode.getByLabel("Node title").fill("Edited scene node");
  await firstNode.getByLabel("Node summary").click();
  await expect(firstNode.getByLabel("Node title")).toHaveValue("Edited scene node");

  await page.getByPlaceholder("Leave a question").fill("Needs a sharper blocker explanation.");
  await page.getByRole("button", { name: "Add comment" }).click();
  await expect(firstNode.locator(".node-comment-row").getByText("Needs a sharper blocker explanation.")).toBeVisible();

  await firstNode.locator(".node-comment-row").click();
  await expect(firstNode.locator(".node-comment-row.is-resolved")).toBeVisible();

  await firstNode.getByRole("button", { name: "Done" }).click();
  await expect(firstNode.getByLabel("Node title")).toHaveCount(0);
  await firstNode.click({ position: { x: 20, y: 20 }, modifiers: ["Alt"] });
  await expect(firstNode.getByLabel("Node title")).toBeVisible();

  const nodeCount = await page.locator(".decision-node").count();
  await firstNode.getByRole("button", { name: "Evidence" }).click();
  await expect.poll(async () => page.locator(".decision-node").count()).toBeGreaterThan(nodeCount);
  await firstNode.click({ button: "right" });
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByRole("menuitem", { name: "Bring to front" }).click();

  await firstNode.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Copy as Markdown" }).click();
  await firstNode.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Paste copied node" }).click();
  await expect.poll(async () => page.locator(".decision-node").count()).toBeGreaterThan(nodeCount + 1);

  await page.keyboard.press("Escape");
  await expect(page.locator(".canvas-panel")).not.toHaveClass(/has-card-focus/);
  await page.getByRole("button", { name: "MADR Markdown" }).click();
  await expect(page.getByText(/Export created:/)).toBeVisible();
  await expect(page.locator(".export-preview")).toBeVisible();
  await expect(page.locator(".export-preview-body")).toContainText("## Context and Problem Statement");
  await expect(page.getByRole("button", { name: "Copy" })).toBeVisible();
  await expect(page.locator(".artifact-strip a").first()).toBeVisible();
});
