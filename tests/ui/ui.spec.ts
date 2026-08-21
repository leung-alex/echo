import { expect, test } from "@playwright/test";

import { installEchoFixture } from "./echo-fixture";

test.beforeEach(async ({ page }) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByRole("tab", { name: "History" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
});

test("keeps History, Favorites, direct actions, and settings on one Echo surface", async ({
  page,
}) => {
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("No favorites yet")).toBeVisible();

  await page.getByRole("tab", { name: "History" }).click();
  await expect(page.getByRole("button", { name: "Pin" }).first()).toBeVisible();
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await expect(page.getByText("Added to Favorites")).toBeVisible();
  await expect(page.getByText("Alpha clipboard")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        !(
          window as Window & {
            __echoMockState?: { calls: Array<{ command: string }> };
          }
        ).__echoMockState?.calls.some(
          (call) => call.command === "quick_insert_execute",
        ),
    ),
  ).toBe(true);
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  await page.getByRole("button", { name: "Edit saved item" }).click();
  await expect(
    page.getByRole("dialog", { name: "Edit favorite" }),
  ).toContainText("Content");
  await expect(page.getByLabel("Choose favorite icon")).toBeVisible();
  await page.getByRole("button", { name: "Close favorite editor" }).click();

  await page.getByRole("tab", { name: "History" }).click();
  await page.getByRole("button", { name: "Copy" }).first().click();
  await expect(page.getByText("Copied")).toBeVisible();
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Clipboard Settings" }),
  ).toBeVisible();
  await page.getByRole("switch", { name: "Record sensitive content" }).click();
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByText("Settings updated")).toBeVisible();
});

test("renders History batch selection affordances without row insertion", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Select" }).click();
  await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  await expect(
    page.getByRole("toolbar", { name: "History batch actions" }),
  ).toBeVisible();

  await page.getByRole("row").first().click();
  await expect(page.getByText("1 selected")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Favorite selected" }),
  ).toBeEnabled();

  await page.locator('[data-testid="history-results"]').press("Space");
  await expect(page.getByText("0 selected")).toBeVisible();
  await page.locator('[data-testid="history-results"]').press("Control+A");
  await expect(page.getByText("2 selected")).toBeVisible();
  await page.locator('[data-testid="history-results"]').press("Enter");
  await expect(page.getByText("1 selected")).toBeVisible();
});

test("shows the clear-all confirmation copy and preserves the presentation contract", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Clear all" }).click();
  await expect(page.getByRole("alertdialog")).toContainText(
    "Pinned items and Favorites are preserved",
  );
  await expect(
    page.getByRole("button", { name: "Clear unpinned History" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).last().click();
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
});

test("keeps activation and successful insert behavior recoverable", async ({
  page,
}) => {
  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .press("Escape");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  await page.evaluate(() =>
    (
      window as Window & { __emitEchoActivation?: (payload: unknown) => void }
    ).__emitEchoActivation?.({
      route: "quick_insert",
      query: "Alpha",
      request_id: "reopen-1",
    }),
  );
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .press("Enter");
  await expect(page.getByText("Inserted")).toBeVisible();
});

test("starts native window dragging from the topbar background", async ({
  page,
}) => {
  await page.locator(".clipboard-topbar").click({ position: { x: 4, y: 4 } });
  await expect
    .poll(async () =>
      page.evaluate(() =>
        (
          window as Window & {
            __echoMockState?: { calls: Array<{ command: string }> };
          }
        ).__echoMockState?.calls.some(
          (call) => call.command === "plugin:window|start_dragging",
        ),
      ),
    )
    .toBe(true);

  const dragCalls = await page.evaluate(
    () =>
      (
        window as Window & {
          __echoMockState?: { calls: Array<{ command: string }> };
        }
      ).__echoMockState?.calls.filter(
        (call) => call.command === "plugin:window|start_dragging",
      ).length,
  );
  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .click();
  const dragCallsAfterInput = await page.evaluate(
    () =>
      (
        window as Window & {
          __echoMockState?: { calls: Array<{ command: string }> };
        }
      ).__echoMockState?.calls.filter(
        (call) => call.command === "plugin:window|start_dragging",
      ).length,
  );
  expect(dragCallsAfterInput).toBe(dragCalls);
});

test("does not create markup from rendered content", async ({ page }) => {
  await expect(page.locator("#app script")).toHaveCount(0);
  expect(
    await page
      .locator("#app")
      .evaluate((element) => !element.innerHTML.includes("innerHTML")),
  ).toBe(true);
  await expect(page.locator("[data-testid=clipboard-panel]")).toBeVisible();
});
