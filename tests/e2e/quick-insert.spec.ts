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

  test("pastes successfully, then rejects stale, read-only, unknown, changed, and password targets", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const target = await EchoTargetFixture.start();
    const content = `Echo native target payload ${Date.now()}`;
    let deadTarget: EchoTargetFixture | undefined;
    try {
      await runClipboardFixture("copy-text", content);
      const itemId = await waitForHistoryId(main, content);
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

      // A successful insertion clears the session. Reusing the same command
      // without a new activation must not target the previous control.
      await expect(
        invoke(main, "quick_insert_execute", {
          source: "history",
          id: itemId,
          action: "insert",
        }),
      ).rejects.toThrow("no safe paste target");

      // This is a new activation captured from the read-only EDIT target.
      await target.focusReadOnly();
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: content });
      surface = await waitForMainPage(browser);
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "no safe paste target",
      );
      await expect.poll(() => target.readReadOnly()).toBe("readonly");

      await hideEcho(surface);
      await target.focusUnknown();
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: content });
      surface = await waitForMainPage(browser);
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "no safe paste target",
      );
      await expect
        .poll(() => target.readUnknown())
        .toBe("unknown unsafe target");

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
      const staleClipboardSentinel = `Echo stale target sentinel ${Date.now()}`;
      await runClipboardFixture("copy-text", staleClipboardSentinel);
      await deadTarget.stop();
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "OriginalWindowUnavailable",
      );
      await expect
        .poll(() => runClipboardFixture("read-text"))
        .toBe(staleClipboardSentinel);
    } finally {
      await deadTarget?.stop().catch(() => undefined);
      await target.stop().catch(() => undefined);
      await browser.close();
    }
  });

  test("rejects an elevated target before clipboard staging", async () => {
    test.skip(
      process.env.ECHO_ACCEPTANCE_ELEVATED !== "1",
      "requires the explicit human-approved RunAs acceptance handoff",
    );
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const target = await EchoTargetFixture.startElevated();
    const content = `Echo elevated target payload ${Date.now()}`;
    const clipboardSentinel = `Echo elevated clipboard sentinel ${Date.now()}`;
    try {
      await runClipboardFixture("copy-text", content);
      const itemId = await waitForHistoryId(main, content);
      await runClipboardFixture("copy-text", clipboardSentinel);
      await target.command("focus-primary");
      await expect.poll(() => target.command("primary-focused")).toBe("true");
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: content });
      const surface = await waitForMainPage(browser);
      await surface.getByRole("row", { name: new RegExp(content) }).click();
      await expect(surface.getByRole("alert")).toContainText("ElevatedTarget");
      await expect.poll(() => target.command("read-primary")).toBe("ac");
      await expect
        .poll(() => runClipboardFixture("read-text"))
        .toBe(clipboardSentinel);
      await expect(
        invoke(main, "quick_insert_execute", {
          source: "history",
          id: itemId,
          action: "insert",
        }),
      ).rejects.toThrow("ElevatedTarget");
    } finally {
      await target.stop().catch(() => undefined);
      await browser.close();
    }
  });
});

async function waitForHistoryId(
  page: Parameters<typeof invoke>[0],
  query: string,
): Promise<number> {
  let id: number | undefined;
  await expect
    .poll(async () => {
      const result = await invoke<{ items: Array<{ id: number }> }>(
        page,
        "quick_insert_list",
        { view: "history", query, limit: 20 },
      );
      id = result.items[0]?.id;
      return result.items.length;
    })
    .toBe(1);
  if (id === undefined)
    throw new Error(`history item did not appear: ${query}`);
  return id;
}
