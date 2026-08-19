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
import { EchoTargetFixture } from "./target-fixture";

const execFileAsync = promisify(execFile);

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
        group_name: "acceptance",
      });
      await main.getByRole("tab", { name: "Snippets" }).click();
      await main.getByPlaceholder("Search snippets...").fill(name);
      await expect(main.getByText("Echo acceptance snippet")).toBeVisible();
      await main.getByRole("button", { name: "Copy" }).click();
      await expect(main.getByText("Copied")).toBeVisible();
      await expect
        .poll(() => readClipboardText())
        .toBe("Echo acceptance snippet");

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

  test("pastes into a native target and refuses changed or password targets", async () => {
    const browser = await connectToEcho();
    const main = await waitForMainPage(browser);
    const target = await EchoTargetFixture.start();
    const name = `echo-native-target-${Date.now()}`;
    const content = "Echo native target payload";
    let deadTarget: EchoTargetFixture | undefined;
    try {
      await hideEcho(main);
      await invoke(main, "snippet_save", {
        id: null,
        name,
        content,
        group_name: "acceptance",
      });

      await target.command("focus-primary");
      await expect.poll(() => target.command("primary-focused")).toBe("true");
      await target.allowEchoForeground();
      await target.command("focus-primary");
      await sendActivation("echo.quick_insert", { query: name });
      let surface = await waitForMainPage(browser);
      await surface.getByRole("tab", { name: "Snippets" }).click();
      const row = surface.getByRole("option", { name: new RegExp(name) });
      await expect(row).toBeVisible();
      await row.click();
      await expect
        .poll(() => target.command("read-primary"))
        .toBe(`a${content}c`);

      await target.command("focus-primary");
      await target.allowEchoForeground();
      await sendActivation("echo.quick_insert", { query: name });
      surface = await waitForMainPage(browser);
      await surface.getByRole("tab", { name: "Snippets" }).click();
      await expect(
        surface.getByRole("option", { name: new RegExp(name) }),
      ).toBeVisible();
      await target.command("focus-secondary");
      await surface.getByRole("option", { name: new RegExp(name) }).click();
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
      await sendActivation("echo.quick_insert", { query: name });
      surface = await waitForMainPage(browser);
      await surface.getByRole("tab", { name: "Snippets" }).click();
      await surface.getByRole("option", { name: new RegExp(name) }).click();
      await expect(surface.getByRole("alert")).toContainText(
        "no safe paste target",
      );
      await expect.poll(() => target.command("read-password")).toBe("");

      deadTarget = await EchoTargetFixture.start();
      await deadTarget.command("focus-primary");
      await deadTarget.allowEchoForeground();
      await deadTarget.command("focus-primary");
      await sendActivation("echo.quick_insert", { query: name });
      surface = await waitForMainPage(browser);
      await surface.getByRole("tab", { name: "Snippets" }).click();
      await expect(
        surface.getByRole("option", { name: new RegExp(name) }),
      ).toBeVisible();
      await deadTarget.stop();
      await surface.getByRole("option", { name: new RegExp(name) }).click();
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

async function readClipboardText(): Promise<string> {
  const result = await execFileAsync("powershell.exe", [
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-Sta",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    resolve(process.cwd(), "tests/e2e/clipboard-fixture.ps1"),
    "-Operation",
    "read-text",
  ]);
  return result.stdout.trim();
}
