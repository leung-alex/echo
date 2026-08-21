import { expect, test } from "@playwright/test";

import {
  connectToEcho,
  hideEcho,
  invoke,
  sendActivation,
  waitForMainPage,
} from "./tauri";
import { runClipboardFixture } from "./native-fixture";

test.describe("Echo Clipboard acceptance", () => {
  test.skip(
    process.platform !== "win32" || process.env.ECHO_WINDOWS_ACCEPTANCE !== "1",
    "requires the separately authorized Echo Windows acceptance gate",
  );

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

  test("keeps pinned history under retention pressure and clear all", async () => {
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
        settings: { ...settings, max_entries: 1 },
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
      await expect
        .poll(async () => {
          const items = await listHistory(main, prefix);
          return items.map((item) => item.preview_text);
        })
        .toEqual([replacementValue]);
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
