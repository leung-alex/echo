import { expect, test } from "@playwright/test";

import { connectToEcho, invoke, waitForMainPage } from "./tauri";

test.describe("Echo Quick Insert acceptance", () => {
  test.skip(
    process.platform !== "win32" || process.env.ECHO_WINDOWS_ACCEPTANCE !== "1",
    "requires the separately authorized Echo Windows acceptance gate",
  );

  test("keeps snippet CRUD, search, copy, and invalid-target insertion recoverable", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const name = `echo-snippet-${Date.now()}`;
    let id: number | undefined;
    try {
      id = await invoke<number>(main, "snippet_save", {
        id: null,
        name,
        content: "Echo acceptance snippet",
        groupName: "acceptance",
      });
      await main.getByRole("tab", { name: "Snippets" }).click();
      await main.getByPlaceholder("Search snippets").fill(name);
      await expect(main.getByText("Echo acceptance snippet")).toBeVisible();
      await main.getByRole("button", { name: "Copy" }).click();
      await expect(main.getByText("Copied")).toBeVisible();

      await expect(
        invoke(main, "quick_insert_execute", {
          source: "snippet",
          id,
          action: "insert",
        }),
      ).rejects.toThrow();
      await expect(main.getByText("Echo acceptance snippet")).toBeVisible();
    } finally {
      if (id !== undefined) {
        await invoke(main, "snippet_delete", { id }).catch(() => undefined);
      }
      await browser.close();
    }
  });
});
