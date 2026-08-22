import { expect, test } from "@playwright/test";

import { installEchoFixture } from "./echo-fixture";

const uiFixtureUrl = "http://127.0.0.1:5187/";

test("keeps Favorites order across native pointer, search, reload, restart, and actions", async ({
  browser,
  context,
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();

  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("tab", { name: "Favorites" }).click();

  await expect(page.getByRole("row")).toHaveCount(2);
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await expect(
    page.getByText("Manual order · drag to rearrange"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Edit" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Delete" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Unfavorite|Favorite/ }),
  ).toHaveCount(0);

  // WebView2 can emit pointerdown/dragstart/dragend while omitting the
  // HTML5 dragover/drop pair. Keep this test on the real mouse seam while
  // reproducing that platform event shape; the UI must not depend on drop.
  await page.evaluate(() => {
    document.addEventListener(
      "dragover",
      (event) => event.stopImmediatePropagation(),
      true,
    );
    document.addEventListener(
      "drop",
      (event) => event.stopImmediatePropagation(),
      true,
    );
  });

  const source = page.getByRole("row").nth(1);
  const target = page.getByRole("row").first();
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  expect(sourceBox).not.toBeNull();
  expect(targetBox).not.toBeNull();
  await page.mouse.move(
    sourceBox!.x + sourceBox!.width / 2,
    sourceBox!.y + sourceBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    targetBox!.x + targetBox!.width / 2,
    targetBox!.y + targetBox!.height / 2,
    { steps: 12 },
  );
  await page.mouse.up();
  await expect(page.getByRole("row").first()).toContainText("Alpha clipboard");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = (
          window as unknown as {
            __echoMockState?: {
              favorites: Array<{
                name: string | null;
                preview_text: string | null;
              }>;
              calls: Array<{ command: string; args: Record<string, unknown> }>;
            };
          }
        ).__echoMockState;
        return {
          order: state?.favorites.map(
            (item) => item.name ?? item.preview_text ?? "",
          ),
          reorderCalls: state?.calls.filter(
            (call) => call.command === "quick_insert_reorder_favorites",
          ).length,
        };
      }),
    )
    .toEqual({
      order: ["Alpha clipboard", "Image clipboard"],
      reorderCalls: 1,
    });

  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .fill("clipboard");
  await expect(
    page.getByText("Search relevance · drag reorder disabled"),
  ).toBeVisible();
  await expect(page.locator(".echo-search-match").first()).toContainText(
    "clipboard",
  );

  // Search mode must reject the same genuine pointer gesture, rather than
  // relying on an HTML draggable attribute that is always false.
  const filteredRows = page.getByRole("row");
  await expect(filteredRows).toHaveCount(2);
  const filteredSource = filteredRows.nth(1);
  const filteredTarget = filteredRows.first();
  const filteredSourceBox = await filteredSource.boundingBox();
  const filteredTargetBox = await filteredTarget.boundingBox();
  expect(filteredSourceBox).not.toBeNull();
  expect(filteredTargetBox).not.toBeNull();
  await page.mouse.move(
    filteredSourceBox!.x + filteredSourceBox!.width / 2,
    filteredSourceBox!.y + filteredSourceBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    filteredTargetBox!.x + filteredTargetBox!.width / 2,
    filteredTargetBox!.y + filteredTargetBox!.height / 2,
    { steps: 12 },
  );
  await page.mouse.up();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = (
          window as unknown as {
            __echoMockState?: {
              favorites: Array<{
                name: string | null;
                preview_text: string | null;
              }>;
              calls: Array<{ command: string; args: Record<string, unknown> }>;
            };
          }
        ).__echoMockState;
        return {
          order: state?.favorites.map(
            (item) => item.name ?? item.preview_text ?? "",
          ),
          reorderCalls: state?.calls.filter(
            (call) => call.command === "quick_insert_reorder_favorites",
          ).length,
        };
      }),
    )
    .toEqual({
      order: ["Alpha clipboard", "Image clipboard"],
      reorderCalls: 1,
    });

  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .fill("");
  const firstFavorite = page.getByRole("row").first();
  await firstFavorite.click();
  await expect(firstFavorite).toHaveAttribute("aria-selected", "true");
  await expect(firstFavorite).toContainText("Alpha clipboard");

  await firstFavorite.getByRole("button", { name: "Copy" }).click();
  await expect(page.getByText("Copied")).toBeVisible();
  await expect(page.getByRole("row").first()).toContainText("Alpha clipboard");

  await page.reload();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByRole("row").first()).toContainText("Alpha clipboard");

  // Recreate the browser context from persisted storage to cover the
  // restart boundary, not just a same-context reload.
  const restartedContext = await browser.newContext({
    storageState: await context.storageState(),
  });
  const restartedPage = await restartedContext.newPage();
  try {
    await installEchoFixture(restartedPage);
    await restartedPage.goto(uiFixtureUrl);
    await expect(restartedPage).toHaveURL(uiFixtureUrl);
    await restartedPage.getByRole("tab", { name: "Favorites" }).click();
    await expect(restartedPage.getByRole("row").first()).toContainText(
      "Alpha clipboard",
    );
  } finally {
    await restartedContext.close();
  }

  await page.evaluate(() =>
    (
      window as unknown as {
        __emitEchoActivation?: (payload: unknown) => void;
      }
    ).__emitEchoActivation?.({
      route: "quick_insert",
      query: "",
      request_id: "favorites-order-insert-1",
    }),
  );
  await expect(
    page.locator('[data-runtime-context="quick-insert"]'),
  ).toHaveCount(1);
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("row").first().click();
  await expect(page.getByText("Inserted")).toBeVisible();
  await expect(page.getByRole("row").first()).toContainText("Alpha clipboard");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const state = (
          window as unknown as {
            __echoMockState?: {
              favorites: Array<{
                name: string | null;
                preview_text: string | null;
              }>;
            };
          }
        ).__echoMockState;
        return state?.favorites.map(
          (item) => item.name ?? item.preview_text ?? "",
        );
      }),
    )
    .toEqual(["Alpha clipboard", "Image clipboard"]);
});

test("opens the create path and Apps SDK icon picker with keyboard affordances", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("button", { name: "Create favorite" }).click();

  const editor = page.getByRole("dialog", { name: "Create favorite" });
  await expect(editor).toContainText("Content");
  await expect(editor).toContainText("Name");
  await expect(editor).toContainText("Icon");
  await expect(editor).toContainText("Tags");
  await editor.getByRole("button", { name: "Choose favorite icon" }).click();

  const picker = page.getByRole("dialog", { name: "Choose favorite icon" });
  await expect(picker).toBeVisible();
  await expect(picker.getByRole("option", { name: "No icon" })).toBeVisible();
  await picker
    .getByPlaceholder("Search the full icon catalog")
    .fill("terminal");
  await expect(
    picker.getByRole("option", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await picker
    .getByPlaceholder("Search the full icon catalog")
    .press("ArrowDown");
  await picker.getByPlaceholder("Search the full icon catalog").press("Enter");
  await expect(editor.getByText("Terminal")).toBeVisible();
});

test("requires non-empty Favorite Content with an accessible focused error", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("button", { name: "Create favorite" }).click();

  const editor = page.getByRole("dialog", { name: "Create favorite" });
  const content = editor.getByRole("textbox", { name: "Content" });
  await expect(content).toHaveAttribute("aria-required", "true");
  await content.fill("   ");
  await editor.getByRole("button", { name: "Save Favorite" }).click();

  await expect(content).toHaveAttribute("aria-invalid", "true");
  await expect(content).toHaveAttribute(
    "aria-describedby",
    "favorite-content-error",
  );
  await expect(editor.getByRole("alert")).toHaveText("Content is required.");
  await expect(content).toBeFocused();
  await content.fill("Reusable content");
  await expect(content).not.toHaveAttribute("aria-invalid", "true");
  await editor.getByRole("button", { name: "Save Favorite" }).click();
  await expect(editor).toBeHidden();
});
