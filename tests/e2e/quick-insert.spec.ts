import { expect, test } from "@playwright/test";

import {
  connectToEcho,
  hideEcho,
  invoke,
  sendActivation,
  waitForMainPage,
} from "./tauri";
import { runClipboardFixture } from "./native-fixture";
import { EchoTargetFixture } from "./target-fixture";

test.describe("Echo Quick Insert acceptance", () => {
  test.skip(
    process.platform !== "win32" || process.env.ECHO_WINDOWS_ACCEPTANCE !== "1",
    "requires the separately authorized Echo Windows acceptance gate",
  );

  test("keeps history copy and invalid-target insertion recoverable", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const content = `Echo acceptance history ${Date.now()}`;
    try {
      await runClipboardFixture("copy-text", content);
      await expect
        .poll(async () => {
          const page = await invoke<{ items: Array<{ id: number }> }>(
            main,
            "quick_insert_list",
            { view: "history", query: content, limit: 20 },
          );
          return page.items.length;
        })
        .toBe(1);
      const item = main.getByRole("row", { name: new RegExp(content) });
      await expect(item).toBeVisible();
      await item.getByRole("button", { name: "Copy" }).click();
      await expect(main.getByText("Copied")).toBeVisible();
      await expect.poll(() => runClipboardFixture("read-text")).toBe(content);
      await expect(
        invoke(main, "quick_insert_execute", {
          source: "history",
          id: (
            await invoke<{ items: Array<{ id: number }> }>(
              main,
              "quick_insert_list",
              {
                view: "history",
                query: content,
                limit: 20,
              },
            )
          ).items[0]?.id,
          action: "insert",
        }),
      ).rejects.toThrow();
      await expect(item).toBeVisible();
    } finally {
      await browser.close();
    }
  });

  test("pastes into a native target and refuses changed or password targets", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const target = await EchoTargetFixture.start();
    const content = `Echo native target payload ${Date.now()}`;
    let deadTarget: EchoTargetFixture | undefined;
    try {
      await runClipboardFixture("copy-text", content);
      await target.command("focus-primary");
      await expect.poll(() => target.command("primary-focused")).toBe("true");
      await target.allowEchoForeground();
      await target.command("focus-primary");
      await sendActivation("echo.quick_insert", { query: content });
      let surface = await waitForMainPage(browser);
      const row = surface.getByRole("row", { name: new RegExp(content) });
      await expect(row).toBeVisible();
      await row.click();
      await expect
        .poll(() => target.command("read-primary"))
        .toBe(`a${content}c`);

      await target.command("focus-primary");
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: content });
      surface = await waitForMainPage(browser);
      await expect(
        surface.getByRole("row", { name: new RegExp(content) }),
      ).toBeVisible();
      await target.command("focus-secondary");
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "InputUnavailable",
      );
      await expect
        .poll(() => target.command("read-primary"))
        .toBe(`a${content}c`);
      await expect.poll(() => target.command("read-secondary")).toBe("");

      await hideEcho(surface);
      await target.command("focus-password");
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: content });
      surface = await waitForMainPage(browser);
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "no safe paste target",
      );
      await expect.poll(() => target.command("read-password")).toBe("");

      deadTarget = await EchoTargetFixture.start();
      await deadTarget.command("focus-primary");
      await deadTarget.allowEchoForeground();
      await deadTarget.command("focus-primary");
      await sendActivation("echo.quick_insert", { query: content });
      surface = await waitForMainPage(browser);
      await expect(
        surface.getByRole("row", { name: new RegExp(content) }),
      ).toBeVisible();
      await deadTarget.stop();
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "OriginalWindowUnavailable",
      );
    } finally {
      await deadTarget?.stop().catch(() => undefined);
      await target.stop().catch(() => undefined);
      await browser.close();
    }
  });
});
