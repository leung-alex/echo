import { execFile } from "node:child_process";
import { resolve } from "node:path";
import { promisify } from "node:util";
import { expect, test } from "@playwright/test";

import {
  connectToEcho,
  hideEcho,
  invoke,
  sendActivation,
  waitForMainPage,
} from "./tauri";

const execFileAsync = promisify(execFile);
const fixture = resolve(process.cwd(), "tests/e2e/clipboard-fixture.ps1");

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
          const items = await invoke<
            Array<{ id: number; preview_text?: string }>
          >(main, "quick_insert_list", {
            view: "history",
            query: value,
            limit: 50,
          });
          return items.length;
        })
        .toBe(1);

      const items = await invoke<Array<{ id: number }>>(
        main,
        "quick_insert_list",
        {
          view: "history",
          query: value,
          limit: 50,
        },
      );
      const id = items[0]?.id;
      expect(id).toBeDefined();
      await copyClipboard("copy-text", value);
      await expect
        .poll(
          async () =>
            (
              await invoke<Array<{ id: number }>>(main, "quick_insert_list", {
                view: "history",
                query: value,
                limit: 50,
              })
            ).length,
        )
        .toBe(1);

      await copyClipboard("copy-html", `${value}-html`);
      await copyClipboard("copy-rtf", `${value}-rtf`);
      await copyClipboard("copy-image", `${value}-image`);
      const filePath = await copyClipboard("copy-files", `${value}-file`);
      await copyClipboard("copy-unsupported", `${value}-unsupported`);
      await expect
        .poll(async () => {
          const items = await invoke<Array<{ content_type: string }>>(
            main,
            "quick_insert_list",
            { view: "history", query: "", limit: 50 },
          );
          return {
            html: items.filter((item) => item.content_type === "html").length,
            rtf: items.filter((item) => item.content_type === "rtf").length,
            image: items.filter((item) => item.content_type === "image").length,
            files: items.filter((item) => item.content_type === "files").length,
          };
        })
        .toEqual({ html: 1, rtf: 1, image: 1, files: 1 });
      expect(filePath).toContain("echo-clipboard-");

      const search = main.getByPlaceholder("Search history");
      await search.fill(value);
      await expect(main.getByText(value)).toBeVisible();
      await main.getByRole("button", { name: "Favorite" }).click();
      await expect(main.getByText("Added to Favorites")).toBeVisible();
      await invoke(main, "history_clear");
      await main.getByRole("tab", { name: "Favorites" }).click();
      await expect(main.getByText(value)).toBeVisible();
      await hideEcho(main);
      await sendActivation("echo.open");
      await expect(
        main.getByRole("heading", { name: "History" }),
      ).toBeVisible();
    } finally {
      await invoke(main, "history_clear").catch(() => undefined);
      await browser.close();
    }
  });
});

async function copyClipboard(
  operation: string,
  value: string,
): Promise<string> {
  const result = await execFileAsync("powershell.exe", [
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-Sta",
    "-File",
    fixture,
    "-Operation",
    operation,
    "-Value",
    value,
  ]);
  return result.stdout.trim();
}
