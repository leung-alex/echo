import { chromium, type Browser, type Page } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

export type EchoBrowser = Browser;
const execFileAsync = promisify(execFile);

export function acceptanceCdpUrl(): string {
  const port = Number(process.env.ECHO_ACCEPTANCE_CDP_PORT ?? "59224");
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    throw new Error(`invalid ECHO_ACCEPTANCE_CDP_PORT: ${port}`);
  }
  return `http://127.0.0.1:${port}`;
}

export async function connectToEcho(): Promise<EchoBrowser> {
  const browser = await chromium.connectOverCDP(acceptanceCdpUrl());
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (await findMainPage(browser)) return browser;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  await browser.close();
  throw new Error("Echo main WebView did not become available");
}

export async function waitForMainPage(browser: EchoBrowser): Promise<Page> {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const page = await findMainPage(browser);
    if (page) return page;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error("Echo main page did not become available");
}

export async function invoke<T = unknown>(
  page: Page,
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  const response = await page.evaluate(
    async ({ command: name, args: input }) => {
      const internals = (
        window as typeof window & {
          __TAURI_INTERNALS__?: {
            invoke?: (
              command: string,
              args: Record<string, unknown>,
            ) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__;
      if (typeof internals?.invoke !== "function") {
        throw new Error("Echo acceptance requires Tauri IPC");
      }
      return internals.invoke(name, input);
    },
    { command, args },
  );
  if (
    response &&
    typeof response === "object" &&
    "status" in response &&
    response.status === "error"
  ) {
    throw new Error(String("error" in response ? response.error : response));
  }
  return response as T;
}

export async function hideEcho(page: Page): Promise<void> {
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
}

export async function sendActivation(
  action: "echo.open" | "echo.quick_insert" | "echo.settings",
  payload: Record<string, unknown> = {},
): Promise<void> {
  const executable = process.env.ECHO_ACCEPTANCE_EXE;
  if (!executable) {
    throw new Error(
      "ECHO_ACCEPTANCE_EXE is required for activation acceptance",
    );
  }
  const envelope = {
    version: 1,
    request_id: randomUUID(),
    action,
    origin: { platform: "windows" },
    payload,
  };
  const encoded = Buffer.from(JSON.stringify(envelope)).toString("base64url");
  await execFileAsync(executable, ["--culsans-activate", encoded], {
    windowsHide: true,
    timeout: 15_000,
  });
}

async function findMainPage(browser: EchoBrowser): Promise<Page | undefined> {
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      if (
        (await page
          .locator("#app")
          .count()
          .catch(() => 0)) > 0
      ) {
        return page;
      }
    }
  }
  return undefined;
}
