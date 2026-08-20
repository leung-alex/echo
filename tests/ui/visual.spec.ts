import { expect, test } from "@playwright/test";
import { mkdirSync } from "node:fs";

import { installEchoFixture } from "./echo-fixture";

const evidenceRoot = "test-results/p08-visual";

test.beforeEach(async ({ page }) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
});

test("captures Quick Insert visual parity states", async ({ page }) => {
  mkdirSync(evidenceRoot, { recursive: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.screenshot({
    path: `${evidenceRoot}/quick-insert-history-detailed-1280x720.png`,
    fullPage: true,
  });
  await page.getByRole("button", { name: "Compact view" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/quick-insert-history-compact-1280x720.png`,
    fullPage: true,
  });
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/quick-insert-favorites-1280x720.png`,
    fullPage: true,
  });
  await page.getByRole("tab", { name: "History" }).click();
  await page.screenshot({
    path: `${evidenceRoot}/quick-insert-history-repeat-1280x720.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("captures responsive and settings states without overflow", async ({
  page,
}) => {
  mkdirSync(evidenceRoot, { recursive: true });
  await page.setViewportSize({ width: 680, height: 480 });
  await page.screenshot({
    path: `${evidenceRoot}/quick-insert-history-680x480.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Clipboard Settings" }),
  ).toBeVisible();
  await page.screenshot({
    path: `${evidenceRoot}/clipboard-settings-680x480.png`,
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
