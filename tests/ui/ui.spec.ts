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

test("keeps History, Favorites, Snippets, copy, search, and settings on one Echo surface", async ({
  page,
}) => {
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("No favorites yet")).toBeVisible();

  await page.getByRole("tab", { name: "History" }).click();
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await expect(page.getByText("Added to Favorites")).toBeVisible();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("Alpha clipboard")).toBeVisible();

  await page.getByRole("tab", { name: "Snippets" }).click();
  await expect(page.getByText("Hello from Echo")).toBeVisible();
  await page.getByRole("combobox", { name: "Search snippets" }).fill("Hello");
  await expect(page.getByText("Hello from Echo")).toBeVisible();

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

test("creates and deletes a snippet through Echo settings", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: "New snippet" }).click();
  await page
    .getByRole("textbox", { name: "Snippet name" })
    .fill("Saved from UI");
  await page
    .getByRole("textbox", { name: "Snippet content" })
    .fill("UI snippet body");
  await page.getByRole("button", { name: "Save snippet" }).click();
  await expect(page.getByText("UI snippet body")).toBeVisible();
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "Delete Saved from UI" }).click();
  await expect(page.getByText("UI snippet body")).toHaveCount(0);
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

  const dragCalls = await page.evaluate(() =>
    (
      window as Window & {
        __echoMockState?: { calls: Array<{ command: string }> };
      }
    ).__echoMockState?.calls.filter(
      (call) => call.command === "plugin:window|start_dragging",
    ).length,
  );
  await page.getByRole("combobox", { name: "Search clipboard history" }).click();
  const dragCallsAfterInput = await page.evaluate(() =>
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
