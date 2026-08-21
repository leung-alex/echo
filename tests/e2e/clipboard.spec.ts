import { expect, test, type Page } from "@playwright/test";

import {
  connectToEcho,
  hideEcho,
  invoke,
  sendActivation,
  waitForFavoritesPage,
  waitForMainPage,
} from "./tauri";
import { runClipboardFixture } from "./native-fixture";

test.describe("Echo Clipboard acceptance", () => {
  test.skip(
    process.platform !== "win32" || process.env.ECHO_WINDOWS_ACCEPTANCE !== "1",
    "requires the separately authorized Echo Windows acceptance gate",
  );

  test("reports honest Mica capability on both native windows", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const favorites = await waitForFavoritesPage(browser);
    const forceFallback =
      process.env.ECHO_ACCEPTANCE_FORCE_MICA_FALLBACK === "1";
    const settings = await invoke<ClipboardSettings>(main, "settings_get");
    try {
      await invoke(main, "settings_update", { settings });
      const expectedMica = forceFallback ? "fallback" : "native";
      await expect
        .poll(async () => {
          const surfaces = await Promise.all(
            [main, favorites].map((page) => readMicaSurface(page)),
          );
          return surfaces.map((surface) => surface.mica);
        })
        .toEqual([expectedMica, expectedMica]);

      const surfaces = await Promise.all(
        [main, favorites].map((page) => readMicaSurface(page)),
      );
      for (const surface of surfaces) {
        expect(surface.hasFakeBlur).toBe(false);
        expect(surface.backdropFilter).toBe("none");
        if (forceFallback) {
          expect(surface.backgroundOpaque).toBe(true);
          expect(surface.readable).toBe(true);
        }
      }
    } finally {
      await browser.close();
    }
  });

  test("captures text, deduplicates it, keeps favorites after clearing history, and reopens hidden UI", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const value = `echo-clipboard-${Date.now()}`;
    try {
      await invoke(main, "history_clear");
      await copyClipboard("copy-text", value);
      await expect
        .poll(async () => {
          const page = await invoke<{
            items: Array<{ id: number; preview_text?: string }>;
          }>(main, "quick_insert_list", {
            view: "history",
            query: value,
            limit: 50,
          });
          return page.items.length;
        })
        .toBe(1);

      const page = await invoke<{ items: Array<{ id: number }> }>(
        main,
        "quick_insert_list",
        {
          view: "history",
          query: value,
          limit: 50,
        },
      );
      const id = page.items[0]?.id;
      expect(id).toBeDefined();
      await copyClipboard("copy-text", value);
      await expect
        .poll(
          async () =>
            (
              await invoke<{ items: Array<{ id: number }> }>(
                main,
                "quick_insert_list",
                {
                  view: "history",
                  query: value,
                  limit: 50,
                },
              )
            ).items.length,
        )
        .toBe(1);

      await copyClipboard("copy-html", `${value}-html`);
      await copyClipboard("copy-rtf", `${value}-rtf`);
      await copyClipboard("copy-image", `${value}-image`);
      const filePath = await copyClipboard("copy-files", `${value}-file`);
      await copyClipboard("copy-unsupported", `${value}-unsupported`);
      await expect
        .poll(async () => {
          const page = await invoke<{ items: Array<{ content_type: string }> }>(
            main,
            "quick_insert_list",
            { view: "history", query: "", limit: 50 },
          );
          return {
            html: page.items.filter((item) => item.content_type === "html")
              .length,
            rtf: page.items.filter((item) => item.content_type === "rtf")
              .length,
            image: page.items.filter((item) => item.content_type === "image")
              .length,
            files: page.items.filter((item) => item.content_type === "files")
              .length,
          };
        })
        .toEqual({ html: 1, rtf: 1, image: 1, files: 1 });
      expect(filePath).toContain("echo-clipboard-");

      const search = main.getByPlaceholder("Search clipboard history...");
      await search.fill(value);
      const escapedValue = value.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&");
      const originalEntry = main.getByRole("row", {
        name: new RegExp(`^${escapedValue}(?:\\s|$)`),
      });
      await expect(originalEntry).toBeVisible();
      await originalEntry.hover();
      await originalEntry.getByRole("button", { name: "Favorite" }).click();
      await expect(main.getByText("Added to Favorites")).toBeVisible();
      await invoke(main, "history_clear");
      await main.getByRole("tab", { name: "Favorites" }).click();
      await expect(
        main.getByRole("row", {
          name: new RegExp(`^${escapedValue}(?:\\s|$)`),
        }),
      ).toBeVisible();
      await hideEcho(main);
      await sendActivation("echo.open");
      const reopenedBrowser = await connectToEcho();
      try {
        const reopened = await waitForMainPage(reopenedBrowser);
        await expect(
          reopened.getByRole("tab", { name: "History" }),
        ).toHaveAttribute("aria-selected", "true");
      } finally {
        await reopenedBrowser.close();
      }
    } finally {
      await invoke(main, "history_clear").catch(() => undefined);
      await browser.close();
    }
  });

  test("retains pinned history through pressure and restores eviction after unpin", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const prefix = `echo-retention-${Date.now()}`;
    const pinnedValue = `${prefix}-pinned`;
    const transientValue = `${prefix}-transient`;
    const replacementValue = `${prefix}-replacement`;
    const settings = await invoke<ClipboardSettings>(main, "settings_get");
    let pinnedId: number | undefined;
    try {
      await invoke(main, "history_clear");
      await invoke(main, "settings_update", {
        settings: { ...settings, max_entries: 2 },
      });

      await copyClipboard("copy-text", pinnedValue);
      pinnedId = await waitForHistoryId(main, pinnedValue);
      await expect(
        invoke(main, "quick_insert_pin_history", { id: pinnedId }),
      ).resolves.toBe(true);

      await copyClipboard("copy-text", transientValue);
      await expect
        .poll(async () => {
          const items = await listHistory(main, prefix);
          return items.map((item) => item.preview_text);
        })
        .toEqual([pinnedValue, transientValue]);

      await invoke(main, "settings_update", {
        settings: { ...settings, max_entries: 1 },
      });
      await copyClipboard("copy-text", `${prefix}-pressure`);
      await expect
        .poll(async () => {
          const items = await listHistory(main, prefix);
          return items.map((item) => item.preview_text);
        })
        .toEqual([pinnedValue]);

      await invoke(main, "history_clear");
      await expect
        .poll(async () => {
          const items = await listHistory(main, prefix);
          return items.map((item) => item.preview_text);
        })
        .toEqual([pinnedValue]);

      await expect(
        invoke(main, "quick_insert_unpin_history", { id: pinnedId }),
      ).resolves.toBe(true);
      await copyClipboard("copy-text", replacementValue);
      const replacementId = await waitForHistoryId(main, replacementValue);
      await expect
        .poll(async () => {
          const items = await listHistory(main, prefix);
          return items.map((item) => item.preview_text);
        })
        .toEqual([replacementValue]);

      await expect(
        invoke(main, "quick_insert_delete_history_many", {
          request: { ids: [replacementId] },
        }),
      ).resolves.toBe(1);
      await expect.poll(() => listHistory(main, prefix)).toEqual([]);
    } finally {
      if (pinnedId !== undefined) {
        await invoke(main, "quick_insert_unpin_history", {
          id: pinnedId,
        }).catch(() => undefined);
      }
      await invoke(main, "history_clear").catch(() => undefined);
      await invoke(main, "settings_update", { settings }).catch(
        () => undefined,
      );
      await browser.close();
    }
  });
});

type ClipboardSettings = {
  history_enabled: boolean;
  record_sensitive: boolean;
  store_window_titles: boolean;
  max_entries: number;
  max_total_bytes: number;
  max_item_bytes: number;
  theme: "system" | "light" | "dark";
};

type HistoryItem = {
  id: number;
  preview_text?: string;
  pinned_at?: number;
};

type MicaSurface = {
  mica: string | null;
  backgroundOpaque: boolean;
  readable: boolean;
  backdropFilter: string;
  hasFakeBlur: boolean;
};

async function readMicaSurface(page: Page): Promise<MicaSurface> {
  return page.evaluate(() => {
    const panel = document.querySelector<HTMLElement>(".clipboard-window");
    if (!panel) throw new Error("Echo surface is missing");
    const style = getComputedStyle(panel);
    const background = style.backgroundColor;
    const channels = background.match(/^rgba?\((.*)\)$/)?.[1]?.split(",");
    const alpha = channels?.length === 4 ? Number(channels[3]) : 1;
    const cssText = Array.from(document.styleSheets)
      .flatMap((sheet) => {
        try {
          return Array.from(sheet.cssRules, (rule) => rule.cssText);
        } catch {
          return [];
        }
      })
      .join("\n");
    return {
      mica: document.documentElement.dataset.mica ?? null,
      backgroundOpaque: background !== "transparent" && alpha > 0,
      readable: style.color !== "transparent" && style.color !== background,
      backdropFilter: style.getPropertyValue("backdrop-filter") || "none",
      hasFakeBlur: /(?:-webkit-)?backdrop-filter\s*:/i.test(cssText),
    };
  });
}

async function listHistory(page: Parameters<typeof invoke>[0], query: string) {
  const result = await invoke<{ items: HistoryItem[] }>(
    page,
    "quick_insert_list",
    { view: "history", query, limit: 50 },
  );
  return result.items;
}

async function waitForHistoryId(
  page: Parameters<typeof invoke>[0],
  query: string,
): Promise<number> {
  let id: number | undefined;
  await expect
    .poll(async () => {
      const items = await listHistory(page, query);
      id = items[0]?.id;
      return items.length;
    })
    .toBe(1);
  if (id === undefined)
    throw new Error(`history item did not appear: ${query}`);
  return id;
}

async function copyClipboard(
  operation: string,
  value: string,
): Promise<string> {
  return runClipboardFixture(operation, value);
}
