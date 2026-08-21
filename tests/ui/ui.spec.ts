import { expect, test } from "@playwright/test";

import { installEchoFixture } from "./echo-fixture";
import { readInlineImageActionState } from "./inline-image-actions";

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
  const createFavorite = page.getByRole("button", { name: "Create favorite" });
  await expect(createFavorite).toHaveAttribute("type", "button");
  await expect(createFavorite).toHaveAttribute("aria-label", "Create favorite");
  await expect(createFavorite).toHaveText("");

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
  await page.getByRole("button", { name: "Edit" }).click();
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

test("states the clear-history preservation policy in the confirmation copy", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Clipboard Settings" }),
  ).toBeVisible();

  const dialogMessage = new Promise<string>((resolve) => {
    page.once("dialog", async (dialog) => {
      resolve(dialog.message());
      await dialog.dismiss();
    });
  });
  await page.getByRole("button", { name: "Clear history" }).click();
  await expect(await dialogMessage).toContain(
    "Pinned History items and Favorites are preserved.",
  );
});

test("applies typed theme capability events without inventing initial Mica state", async ({
  page,
}) => {
  const root = page.locator("html");
  await expect(root).not.toHaveAttribute("data-mica", "fallback");

  await page.evaluate(() =>
    (
      window as Window & {
        __emitEchoThemeChanged?: (payload: unknown) => void;
      }
    ).__emitEchoThemeChanged?.({ mode: "light", nativeMica: true }),
  );
  await expect(root).toHaveAttribute("data-theme", "light");
  await expect(root).toHaveAttribute("data-mica", "native");
});

test("keeps inline image actions non-interactive until fully visible", async ({
  page,
}) => {
  const row = page.getByRole("row").nth(1);
  const rail = row.locator(".echo-image-actions .echo-row-actions");
  const favoriteAction = rail.locator(
    'button[data-testid="history-favorite-action"]',
  );

  await page.mouse.move(0, 0);
  await expect(rail).toHaveCSS("visibility", "hidden");
  await expect(rail).toHaveCSS("opacity", "0");
  await expect(rail).toHaveCSS("pointer-events", "none");
  await expect(favoriteAction).toHaveAttribute("tabindex", "-1");

  await row.hover();
  await expect(rail).toHaveCSS("visibility", "visible");
  await expect(rail).toHaveCSS("opacity", "1");
  await expect(rail).toHaveCSS("pointer-events", "auto");
  let state = await readInlineImageActionState(rail);
  expect(state.actionCount).toBeGreaterThan(0);
  expect(state.transitionProperty).toBe("none");
  expect(state.minContrast).toBeGreaterThanOrEqual(4.5);

  await row.focus();
  await expect(row).toBeFocused();
  await expect(favoriteAction).toHaveAttribute("tabindex", "0");
  await page.keyboard.press("Tab");
  await expect(favoriteAction).toBeFocused();
  state = await readInlineImageActionState(rail);
  expect(state.visibility).toBe("visible");
  expect(state.opacity).toBe("1");
  expect(state.pointerEvents).toBe("auto");
  expect(state.minContrast).toBeGreaterThanOrEqual(4.5);

  await favoriteAction.hover();
  state = await readInlineImageActionState(rail);
  expect(state.minContrast).toBeGreaterThanOrEqual(4.5);

  await page.emulateMedia({ reducedMotion: "reduce" });
  await row.hover();
  await expect(rail).toHaveCSS("transition-property", "none");
  state = await readInlineImageActionState(rail);
  expect(state.visibility).toBe("visible");
  expect(state.opacity).toBe("1");
  expect(state.pointerEvents).toBe("auto");
  expect(state.minContrast).toBeGreaterThanOrEqual(4.5);
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

test("keeps manager row clicks separate from Quick Insert row insertion", async ({
  page,
}) => {
  const row = page.getByRole("row").first();
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
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

  await page.evaluate(() =>
    (
      window as Window & { __emitEchoActivation?: (payload: unknown) => void }
    ).__emitEchoActivation?.({
      route: "quick_insert",
      query: "Alpha",
      request_id: "click-insert-1",
    }),
  );
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
  await page.getByRole("row").first().click();
  await expect(page.getByText("Inserted")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (
        window as Window & {
          __echoMockState?: {
            calls: Array<{ command: string; args: Record<string, unknown> }>;
          };
        }
      ).__echoMockState?.calls.some(
        (call) =>
          call.command === "quick_insert_execute" &&
          call.args.action === "insert",
      ),
    ),
  ).toBe(true);
});

test("makes selected row actions keyboard reachable without executing the row", async ({
  page,
}) => {
  const search = page.getByRole("combobox", {
    name: "Search clipboard history",
  });
  const row = page.getByRole("row").first();

  await search.press("ArrowUp");
  await expect(row).toHaveAttribute("aria-selected", "true");
  await expect(row).toHaveAttribute("tabindex", "0");
  await expect(row.getByRole("button", { name: "Favorite" })).toHaveAttribute(
    "tabindex",
    "0",
  );

  await row.focus();
  await expect(row).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(row.getByRole("button", { name: "Favorite" })).toBeFocused();
  await page.keyboard.press("Enter");

  await expect(page.getByText("Added to Favorites")).toBeVisible();
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
});

test("keeps variable-height rows separated through resize and filtering", async ({
  page,
}) => {
  await page.evaluate(() => {
    const windowWithEcho = window as Window & {
      __echoMockState?: {
        history: Array<{
          preview_text: string | null;
          editable_text: string | null;
        }>;
      };
      __emitEchoHistoryChanged?: () => void;
    };
    const longText = Array.from(
      { length: 18 },
      (_, index) => `Variable row line ${index + 1}`,
    ).join("\n");
    const first = windowWithEcho.__echoMockState?.history[0];
    if (first) {
      first.preview_text = longText;
      first.editable_text = longText;
    }
    windowWithEcho.__emitEchoHistoryChanged?.();
  });

  await expect(page.getByText("Variable row line 18")).toBeVisible();
  const getRowGeometry = () =>
    page.locator('[data-virtualized-row="true"]').evaluateAll((rows) =>
      rows.map((row) => {
        const rect = row.getBoundingClientRect();
        return { top: rect.top, bottom: rect.bottom, height: rect.height };
      }),
    );

  await expect
    .poll(async () => {
      const rows = await getRowGeometry();
      return (
        rows.length === 2 &&
        rows[0].height > 200 &&
        rows[1].top >= rows[0].bottom - 1
      );
    })
    .toBe(true);

  await page.setViewportSize({ width: 420, height: 360 });
  await expect
    .poll(async () => {
      const rows = await getRowGeometry();
      return rows.length === 2 && rows[1].top >= rows[0].bottom - 1;
    })
    .toBe(true);

  const search = page.getByRole("combobox", {
    name: "Search clipboard history",
  });
  await search.fill("Variable row line 18");
  await expect(page.getByRole("row")).toHaveCount(1);
  await search.fill("");
  await expect(page.getByRole("row")).toHaveCount(2);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("traps editor and icon-picker focus and restores the launcher focus", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("button", { name: "Edit" }).click();

  const editor = page.getByRole("dialog", { name: "Edit favorite" });
  await expect(editor).toBeVisible();
  await expect(page.locator("#favorite-content")).toBeFocused();

  const iconField = page.getByRole("button", { name: "Choose favorite icon" });
  await iconField.click();
  const picker = page.getByRole("dialog", { name: "Choose favorite icon" });
  const iconSearch = picker.getByPlaceholder("Search the full icon catalog");
  await expect(iconSearch).toBeFocused();
  await iconSearch.press("ArrowDown");
  await expect(iconSearch).toHaveAttribute(
    "aria-activedescendant",
    /^favorite-icon-option-\d+$/,
  );
  await iconSearch.press("Enter");
  await expect(picker).toHaveCount(0);
  await expect(iconField).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  await expect(page.getByRole("row").first()).toBeFocused();
});

test("disables UI motion under prefers-reduced-motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const styles = await page.evaluate(() => {
    const row = document.querySelector<HTMLElement>(
      '[data-virtualized-row="true"]',
    );
    const caret = document.querySelector<HTMLElement>(
      ".echo-search-field__caret",
    );
    return {
      rowTransition: row ? getComputedStyle(row).transitionProperty : "",
      caretAnimation: caret ? getComputedStyle(caret).animationName : "",
    };
  });
  expect(styles.rowTransition).toBe("none");
  expect(styles.caretAnimation).toBe("none");
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
