import { execFile } from "node:child_process";
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
