import { expect, test } from "@playwright/test";

test("creates a shape graph, comments on a node, and exports markdown", async ({ page }) => {
  await page.goto("/");

  await expect(page.locator(".canvas-watermark").getByText("shape.ai")).toBeVisible();
  await page.getByRole("button", { name: "Open shapes" }).click();
  await expect(page.getByRole("button", { name: "Close shapes" })).toBeVisible();
  await page.getByRole("button", { name: "Create shape" }).click();
  await expect(page.locator(".decision-node").first()).toBeVisible();

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

  await firstNode.getByLabel("Node title").fill("Edited decision node");
  await firstNode.getByLabel("Node summary").click();
  await expect(firstNode.getByLabel("Node title")).toHaveValue("Edited decision node");

  await page.getByPlaceholder("Leave a question").fill("Needs a sharper blocker explanation.");
  await page.getByRole("button", { name: "Add comment" }).click();
  await expect(firstNode.locator(".node-comment-row").getByText("Needs a sharper blocker explanation.")).toBeVisible();

  await firstNode.locator(".node-comment-row").click();
  await expect(firstNode.locator(".node-comment-row.is-resolved")).toBeVisible();

  await firstNode.getByRole("button", { name: "Done" }).click();
  await expect(firstNode.getByLabel("Node title")).toHaveCount(0);
  await firstNode.click({ position: { x: 20, y: 20 }, modifiers: ["Alt"] });
  await expect(firstNode.getByLabel("Node title")).toBeVisible();

  await firstNode.getByRole("button", { name: "Evidence" }).click();
  await expect(page.locator(".decision-node")).toHaveCount(11);
  await firstNode.click({ button: "right" });
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByRole("menuitem", { name: "Bring to front" }).click();

  await firstNode.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Copy as Markdown" }).click();
  await firstNode.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Paste copied node" }).click();
  await expect(page.locator(".decision-node")).toHaveCount(12);
  await expect(page.locator(".floating-inspector")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Toggle comments and details" })).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(page.locator(".canvas-panel")).not.toHaveClass(/has-card-focus/);
  await page.getByRole("button", { name: "MADR Markdown" }).click();
  await expect(page.getByText(/Export created:/)).toBeVisible();
  await expect(page.locator(".artifact-strip a").first()).toBeVisible();
});
