import { expect, test } from "@playwright/test";

test("creates a design graph, comments on a node, and exports markdown", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Charrette" })).toBeVisible();
  await page.getByRole("button", { name: "Create design" }).click();
  await expect(page.locator(".decision-node").first()).toBeVisible();

  await page.locator(".decision-node").first().click();
  await expect(page.getByRole("heading", { name: "Inspector" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Title" })).toBeVisible();

  await page.getByRole("textbox", { name: "Title" }).fill("Edited decision node");
  await page.getByRole("textbox", { name: "Summary" }).click();
  await expect(page.getByRole("textbox", { name: "Title" })).toHaveValue("Edited decision node");

  await page.getByRole("button", { name: "Evidence" }).click();
  await expect(page.locator(".canvas-header").getByText("11 nodes")).toBeVisible();

  await page.getByPlaceholder("Leave a question").fill("Needs a sharper blocker explanation.");
  await page.getByRole("button", { name: "Add comment" }).click();
  await expect(page.locator(".comment-row").getByText("Needs a sharper blocker explanation.")).toBeVisible();

  await page.locator(".comment-row").click();
  await expect(page.locator(".comment-row.is-resolved")).toBeVisible();

  await page.getByRole("button", { name: "AI task plan" }).click();
  await expect(page.getByText(/Export created:/)).toBeVisible();
  await expect(page.locator(".artifact-strip a").first()).toBeVisible();
});
