import { expect, test } from "@playwright/test";
import { mkdirSync } from "node:fs";

import { installEchoFixture } from "./echo-fixture";

const evidenceRoot =
  process.env.ECHO_UI_EVIDENCE_ROOT ?? "test-results/p08-visual";

test.beforeEach(async ({ page }) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  mkdirSync(evidenceRoot, { recursive: true });
});

test("captures History default, hover, selected, batch, image, and search states", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  const panel = page.locator(".clipboard-window");
  const shortcutRail = page.locator(".clipboard-footer");
  await expect(shortcutRail).toHaveCSS("position", "absolute");
  const [panelBox, shortcutRailBox] = await Promise.all([
    panel.boundingBox(),
    shortcutRail.boundingBox(),
  ]);
  expect(panelBox).not.toBeNull();
  expect(shortcutRailBox).not.toBeNull();
  expect(shortcutRailBox!.width).toBeLessThan(panelBox!.width * 0.9);
  await page.screenshot({
    path: `${evidenceRoot}/history-default-light-1280x720.png`,
    fullPage: true,
  });

  await page.getByRole("row").nth(1).hover();
  await page.screenshot({
    path: `${evidenceRoot}/history-hover-image-light-1280x720.png`,
    fullPage: true,
  });

  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .press("ArrowDown");
  await page.screenshot({
    path: `${evidenceRoot}/history-selected-light-1280x720.png`,
    fullPage: true,
  });

  await page.getByRole("button", { name: "Select" }).click();
  await page.getByRole("row").first().click();
  await page.screenshot({
    path: `${evidenceRoot}/history-batch-light-1280x720.png`,
    fullPage: true,
  });

  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .fill("Alpha");
  await page.screenshot({
    path: `${evidenceRoot}/history-search-highlight-light-1280x720.png`,
    fullPage: true,
  });
  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .fill("missing");
  await expect(page.getByText("No matches for “missing”")).toBeVisible();
  await page.screenshot({
    path: `${evidenceRoot}/history-empty-search-light-1280x720.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("captures Favorites, editor, icon picker, dark, and minimum-size states", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/favorites-default-no-icon-light-1280x720.png`,
    fullPage: true,
  });

  await page.getByRole("button", { name: "Create favorite" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/favorite-editor-light-1280x720.png`,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Choose favorite icon" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/favorite-icon-picker-light-1280x720.png`,
    fullPage: true,
  });

  await page.getByRole("button", { name: "Close icon picker" }).click();
  await page.getByRole("button", { name: "Close favorite editor" }).click();
  await page.reload();
  await expect(page.getByRole("tab", { name: "History" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  await expect(page.getByRole("row")).toHaveCount(2);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({
    path: `${evidenceRoot}/history-default-dark-1280x720.png`,
    fullPage: true,
  });

  await page.setViewportSize({ width: 420, height: 360 });
  await page.screenshot({
    path: `${evidenceRoot}/history-min-size-dark-420x360.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
