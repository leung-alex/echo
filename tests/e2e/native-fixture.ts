import { execFile, spawn, type ChildProcess } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export function nativeFixtureExecutable(): string {
  const executable = process.env.ECHO_ACCEPTANCE_FIXTURE_EXE;
  if (!executable) {
    throw new Error(
      "ECHO_ACCEPTANCE_FIXTURE_EXE is required for native acceptance",
    );
  }
  return executable;
}

export async function runClipboardFixture(
  operation: string,
  value = "fixture",
): Promise<string> {
  const result = await execFileAsync(
    nativeFixtureExecutable(),
    ["clipboard", "--operation", operation, "--value", value],
    { windowsHide: true, timeout: 15_000 },
  );
  return result.stdout.trim();
}

export async function holdClipboardFixture(): Promise<{
  stop: () => Promise<void>;
}> {
  const child = spawn(
    nativeFixtureExecutable(),
    ["clipboard", "--operation", "hold-open"],
    { windowsHide: true },
  );
  await waitForClipboardFixtureReady(child);
  return {
    stop: async () => {
      if (child.exitCode !== null) return;
      child.kill();
      await new Promise<void>((resolve) => {
        child.once("exit", () => resolve());
      });
    },
  };
}

async function waitForClipboardFixtureReady(
  child: ChildProcess,
): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(() => {
      child.kill();
      reject(new Error("clipboard holder did not become ready"));
    }, 15_000);
    child.stdout?.on("data", (chunk: Buffer | string) => {
      output += chunk.toString();
      if (output.includes("ready")) {
        clearTimeout(timeout);
        resolve();
      }
    });
    child.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.once("exit", (code) => {
      if (code !== null) {
        clearTimeout(timeout);
        reject(new Error(`clipboard holder exited before ready (${code})`));
      }
    });
  });
}
