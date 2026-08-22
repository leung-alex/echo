import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { isAbsolute, resolve } from "node:path";

import { nativeFixtureExecutable } from "./native-fixture";

type TargetReady = {
  run_id: string;
  process_id: number;
  title: string;
};

type TargetPaths = {
  ready: string;
  command: string;
  response: string;
  primary: string;
  secondary: string;
  password: string;
  readonly: string;
  unknown: string;
};

type TargetLauncher = (
  executable: string,
  runId: string,
  title: string,
  paths: TargetPaths,
) => ChildProcessWithoutNullStreams;

export class EchoTargetFixture {
  private nextRequest = 0;
  private output = "";

  private constructor(
    private readonly child: ChildProcessWithoutNullStreams,
    private readonly runId: string,
    private readonly root: string,
  ) {
    child.stdout.on("data", (chunk) => (this.output += chunk.toString()));
    child.stderr.on("data", (chunk) => (this.output += chunk.toString()));
  }

  static async start(): Promise<EchoTargetFixture> {
    return this.startWithLauncher((executable, runId, title, paths) =>
      spawn(
        executable,
        [
          "target",
          "--run-id",
          runId,
          "--title",
          title,
          "--ready",
          paths.ready,
          "--command",
          paths.command,
          "--response",
          paths.response,
          "--primary",
          paths.primary,
          "--secondary",
          paths.secondary,
          "--password",
          paths.password,
          "--readonly",
          paths.readonly,
          "--unknown",
          paths.unknown,
        ],
        {
          cwd: process.cwd(),
          windowsHide: true,
          env: targetEnvironment(runId, title, paths),
        },
      ),
    );
  }

  /**
   * RunAs is deliberately a human-approved acceptance handoff. The helper is
   * reachable only from the authorized native gate and the explicit elevated
   * opt-in; it never supplies credentials or attempts to answer UAC.
   */
  static async startElevated(): Promise<EchoTargetFixture> {
    if (
      process.platform !== "win32" ||
      process.env.ECHO_WINDOWS_ACCEPTANCE !== "1"
    ) {
      throw new Error(
        "elevated target acceptance requires ECHO_WINDOWS_ACCEPTANCE=1",
      );
    }
    if (process.env.ECHO_ACCEPTANCE_ELEVATED !== "1") {
      throw new Error(
        "set ECHO_ACCEPTANCE_ELEVATED=1 to opt into the human UAC handoff",
      );
    }
    return this.startWithLauncher((executable, runId, title, paths) =>
      spawn(
        executable,
        [
          "target-elevated",
          "--run-id",
          runId,
          "--title",
          title,
          "--ready",
          paths.ready,
          "--command",
          paths.command,
          "--response",
          paths.response,
          "--primary",
          paths.primary,
          "--secondary",
          paths.secondary,
          "--password",
          paths.password,
          "--readonly",
          paths.readonly,
          "--unknown",
          paths.unknown,
        ],
        {
          cwd: process.cwd(),
          windowsHide: true,
          env: targetEnvironment(runId, title, paths),
        },
      ),
    );
  }

  private static async startWithLauncher(
    launch: TargetLauncher,
  ): Promise<EchoTargetFixture> {
    const acceptanceRoot = process.env.ECHO_ACCEPTANCE_RUN_ROOT;
    if (!acceptanceRoot) {
      throw new Error(
        "ECHO_ACCEPTANCE_RUN_ROOT is required for target acceptance",
      );
    }
    const runId = randomUUID();
    const root = resolve(acceptanceRoot, "echo-target-fixture", runId);
    await mkdir(root, { recursive: true });
    const paths: TargetPaths = {
      ready: resolve(root, "ready.json"),
      command: resolve(root, "command.json"),
      response: resolve(root, "response.json"),
      primary: resolve(root, "primary.txt"),
      secondary: resolve(root, "secondary.txt"),
      password: resolve(root, "password.txt"),
      readonly: resolve(root, "readonly.txt"),
      unknown: resolve(root, "unknown.txt"),
    };
    await Promise.all(
      Object.values(paths).map((path) => rm(path, { force: true })),
    );
    const title = `Echo target fixture ${runId}`;
    const executable = nativeFixtureExecutable();
    if (!isAbsolute(executable)) {
      throw new Error("ECHO_ACCEPTANCE_FIXTURE_EXE must be absolute");
    }
    const child = launch(executable, runId, title, paths);
    const fixture = new EchoTargetFixture(child, runId, root);
    try {
      const ready = JSON.parse(
        await fixture.waitForFile(paths.ready, 30_000),
      ) as TargetReady;
      if (
        ready.run_id !== runId ||
        !Number.isSafeInteger(ready.process_id) ||
        ready.process_id <= 0 ||
        ready.title !== title
      ) {
        throw new Error(
          "Echo target fixture identity does not belong to this run",
        );
      }
      return fixture;
    } catch (error) {
      await fixture.stop().catch(() => undefined);
      throw error;
    }
  }

  async command(operation: string, payload = ""): Promise<string> {
    const requestId = `${this.runId}-${process.pid}-${++this.nextRequest}`;
    const command = resolve(this.root, "command.json");
    const temporary = `${command}.tmp`;
    const response = resolve(this.root, "response.json");
    await rm(response, { force: true });
    await writeFile(
      temporary,
      JSON.stringify({
        run_id: this.runId,
        request_id: requestId,
        command: operation,
        payload,
      }),
      "utf8",
    );
    await rename(temporary, command);
    const parsed = JSON.parse(await this.waitForFile(response, 10_000)) as {
      request_id: string;
      value: string;
      error: string;
    };
    if (parsed.request_id !== requestId) {
      throw new Error("Echo target fixture response identity changed");
    }
    const error = Buffer.from(parsed.error, "base64").toString("utf8");
    if (error) {
      throw new Error(
        `Echo target fixture command ${operation} failed: ${error}`,
      );
    }
    return Buffer.from(parsed.value, "base64").toString("utf8");
  }

  async allowEchoForeground(): Promise<void> {
    const echoPid = process.env.ECHO_ACCEPTANCE_PID;
    if (!echoPid)
      throw new Error("ECHO_ACCEPTANCE_PID is required for target acceptance");
    await this.command("allow-foreground", echoPid);
  }

  async focusReadOnly(): Promise<void> {
    await this.command("focus-readonly");
  }

  async readReadOnly(): Promise<string> {
    return this.command("read-readonly");
  }

  async focusUnknown(): Promise<void> {
    await this.command("focus-unknown");
  }

  async readUnknown(): Promise<string> {
    return this.command("read-unknown");
  }

  async stop(): Promise<void> {
    if (this.child.exitCode !== null) {
      if (this.child.exitCode !== 0) {
        throw new Error(`Echo target fixture failed:\n${this.output}`);
      }
      return;
    }
    await this.command("shutdown");
    const deadline = Date.now() + 10_000;
    while (this.child.exitCode === null && Date.now() < deadline) {
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 50));
    }
    if (this.child.exitCode === null) {
      this.child.kill();
      throw new Error(
        `Echo target fixture did not stop cleanly:\n${this.output}`,
      );
    }
    if (this.child.exitCode !== 0) {
      throw new Error(`Echo target fixture failed:\n${this.output}`);
    }
  }

  private async waitForFile(path: string, timeout: number): Promise<string> {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      try {
        return await readFile(path, "utf8");
      } catch {
        if (this.child.exitCode !== null) {
          throw new Error(`Echo target fixture exited early:\n${this.output}`);
        }
        await new Promise((resolveDelay) => setTimeout(resolveDelay, 25));
      }
    }
    throw new Error(`Echo target fixture timed out waiting for ${path}`);
  }
}

function targetEnvironment(
  runId: string,
  title: string,
  paths: TargetPaths,
): NodeJS.ProcessEnv {
  return {
    ...process.env,
    ECHO_TARGET_RUN_ID: runId,
    ECHO_TARGET_TITLE: title,
    ECHO_TARGET_READY: paths.ready,
    ECHO_TARGET_COMMAND: paths.command,
    ECHO_TARGET_RESPONSE: paths.response,
    ECHO_TARGET_PRIMARY_OUTPUT: paths.primary,
    ECHO_TARGET_SECONDARY_OUTPUT: paths.secondary,
    ECHO_TARGET_PASSWORD_OUTPUT: paths.password,
  };
}
