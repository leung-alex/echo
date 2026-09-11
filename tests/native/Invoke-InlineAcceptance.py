"""Authorized Windows inline-completion acceptance with synthetic owned inputs.
Every input driver validates PID, image path, process creation time, and foreground.
This is not evidence for physical IME or applications that were not actually tested.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid
import ctypes
from types import SimpleNamespace
from datetime import datetime, timezone

PAYLOAD = "echo-perf-text-0013 — Reusable content, available when you need it."
QUERY = "echo-perf-text-0013"
TITLE = "Echo Recall"

class NotRun(RuntimeError):
    pass

class ForegroundLease:
    """One foreground executor per Windows logon session, including other runs."""
    def __enter__(self):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateMutexW.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_wchar_p]
        self.kernel.CreateMutexW.restype = ctypes.c_void_p
        self.kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        self.kernel.ReleaseMutex.argtypes = [ctypes.c_void_p]
        self.kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self.handle = self.kernel.CreateMutexW(None, False, r"Local\Echo.Completion.Acceptance.Foreground")
        if not self.handle:
            raise RuntimeError("Could not create the foreground acceptance mutex")
        result = self.kernel.WaitForSingleObject(self.handle, 0)
        if result not in (0, 0x80):
            self.kernel.CloseHandle(self.handle)
            raise RuntimeError("Another Echo foreground acceptance run is active")
        return self

    def __exit__(self, *_):
        self.kernel.ReleaseMutex(self.handle)
        self.kernel.CloseHandle(self.handle)

def atomic(path: Path, value: object) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    # Windows readers may briefly deny replacement. Retry only publication of
    # this same file/request ID; never recreate or execute the command again.
    deadline = time.monotonic() + 1.0
    while True:
        try:
            os.replace(temporary, path)
            return
        except PermissionError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.01)

def source_head(root: Path) -> str:
    """Read local identity without allowing a metadata subprocess to stall tests."""
    try:
        head = (root / ".git" / "HEAD").read_text(encoding="ascii").strip()
        if not head.startswith("ref: "):
            return head
        ref = head[5:]
        loose = root / ".git" / ref
        if loose.exists():
            return loose.read_text(encoding="ascii").strip()
        for line in (root / ".git" / "packed-refs").read_text(encoding="ascii").splitlines():
            if line.endswith(" " + ref):
                return line.split()[0]
    except (OSError, UnicodeError):
        pass
    return "unavailable; executable SHA256 is recorded"

def file_version(path: Path) -> str:
    result = subprocess.run([
        "pwsh", "-NoProfile", "-Command",
        "(Get-Item -LiteralPath $env:ECHO_TEST_VERSION_PATH).VersionInfo.ProductVersion",
    ], env=dict(os.environ, ECHO_TEST_VERSION_PATH=str(path)), capture_output=True,
        text=True, encoding="utf-8", timeout=8, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise RuntimeError("Application version could not be read")
    return result.stdout.strip().lstrip("\ufeff")

class Run:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.root = args.root.resolve()
        self.evidence = args.evidence.resolve()
        if self.evidence.exists():
            raise RuntimeError("Evidence directory must be new")
        self.evidence.mkdir(parents=True)
        marker = json.loads((args.template / "synthetic-fixture.json").read_text(encoding="utf-8-sig"))
        if not marker.get("synthetic") or marker.get("capture_enabled"):
            raise RuntimeError("Only capture-disabled synthetic data may be tested")
        shutil.copytree(args.template, self.evidence / "data")
        self.env = dict(os.environ, ECHO_WINDOWS_ACCEPTANCE="1", ECHO_DATA_DIR=str(self.evidence / "data"),
                        ECHO_ACCEPTANCE_RUN_ROOT=str(self.evidence), ECHO_RENDERER=args.renderer,
                        ECHO_DISABLE_GLOBAL_HOTKEY="0")
        if not args.only or "hidden-reclaim-preserves-inline-insertion" in args.only.split(","):
            self.env["ECHO_MEMORY_TRACE_DIR"] = str(self.evidence)
        if args.native_test:
            self.env["ECHO_NATIVE_TEST_ROOT"] = str(self.evidence)
        else:
            self.env.pop("ECHO_NATIVE_TEST_ROOT", None)
        self.checks: list[dict] = []
        self.records: list[dict] = []
        self.processes: list[subprocess.Popen] = []
        self.echo: subprocess.Popen | None = None
        self.native: subprocess.Popen | None = None
        self.native_title = "Echo Inline Native Fixture " + uuid.uuid4().hex
        self.browser: subprocess.Popen | None = None
        self.browser_title = "Echo Inline Composer Fixture"
        self.logs: list = []
        self.started = datetime.now(timezone.utc).isoformat()
        atomic(self.evidence / "identity.json", {
            "schema": "echo.inline.acceptance.v1", "started": self.started,
            "runner": {"pid": os.getpid(), "executable": sys.executable,
                       "source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()},
            "test_assets": {str(path.resolve()): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in (args.tools / "EchoDriver.exe", args.tools / "EchoInlineDriver.exe",
                                         args.tools / "EchoInlineFixture.exe",
                                         self.root / "tests/native/fixtures/inline-composer.html")},
            "source_snapshot_id": args.source_snapshot_id,
            "executable": str(args.executable.resolve()),
            "sha256": hashlib.sha256(args.executable.read_bytes()).hexdigest(),
            "renderer": args.renderer, "native_test": args.native_test,
            "machine": os.environ.get("COMPUTERNAME"),
            "head": source_head(self.root),
        })

    def launch(self, exe: Path, arguments: list[str], label: str) -> subprocess.Popen:
        print("LAUNCH", label, datetime.now(timezone.utc).isoformat(), flush=True)
        log = (self.evidence / (label + ".log")).open("wb")
        self.logs.append(log)
        # GUI applications still create their normal windows. Console helpers
        # must never create transient foreground windows during input tests.
        process = subprocess.Popen([str(exe), *arguments], env=self.env, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
        self.processes.append(process)
        return process

    def record(self, process: subprocess.Popen, title: str) -> None:
        # Query the actual process image and birth time, not an assumed command path.
        command = (
            f"$p=Get-Process -Id {process.pid}; "
            "[ordered]@{pid=$p.Id;executable=$p.MainModule.FileName;started_utc=$p.StartTime.ToUniversalTime().ToString('o');started_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc()}|ConvertTo-Json -Compress"
        )
        def probe():
            result = subprocess.run(["pwsh", "-NoProfile", "-Command", command], capture_output=True,
                                    text=True, encoding="utf-8", errors="replace", timeout=8,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
            if result.returncode != 0:
                return None
            value = json.loads(result.stdout.lstrip("\ufeff"))
            return value if value.get("executable") else None
        value = self.wait(probe, "actual process image", 10)
        value["title"] = title
        self.records.append(value)
        atomic(self.evidence / "owned-processes.json", self.records)

    def wait(self, probe, description: str, timeout: float = 8):
        end = time.monotonic() + timeout
        last = None
        while time.monotonic() < end:
            try:
                last = probe()
                if last:
                    return last
            except (RuntimeError, FileNotFoundError, PermissionError, json.JSONDecodeError, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(0.03)
        raise RuntimeError(f"Timed out: {description}; last={last}")

    def tool(self, exe: str, arguments: list[str], timeout: float = 15):
        result = subprocess.run([str(self.args.tools / exe), *arguments], env=self.env,
                                capture_output=True, text=True, encoding="utf-8", errors="strict", timeout=timeout,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        value = json.loads(result.stdout.lstrip("\ufeff"))
        if value.get("status") != "PASS":
            raise RuntimeError(str(value))
        return value.get("value")

    def d(self, operation: str, *args):
        assert self.echo is not None
        return self.tool("EchoDriver.exe", [operation, str(self.echo.pid), TITLE, *map(str, args)])

    def f(self, operation: str, process: subprocess.Popen, title: str, *args):
        return self.tool("EchoInlineDriver.exe", [operation, str(self.evidence), str(process.pid), title, *map(str, args)])

    def n(self, operation: str, **arguments):
        request = uuid.uuid4().hex
        atomic(self.evidence / "native-command.json", dict(id=request, op=operation, **arguments))
        def reply():
            value = json.loads((self.evidence / "native-response.json").read_text(encoding="utf-8-sig"))
            if value.get("id") != request:
                return None
            if value.get("status") != "PASS":
                raise RuntimeError(str(value))
            return value["value"]
        return self.wait(reply, "native fixture command " + operation, 6)

    def reset(self, control="single", text="pre| |post", start=4, length=0):
        if self.echo and self.echo.poll() is None and self.d("exists"):
            self.d("close")
            self.wait(lambda: not self.d("exists"), "Echo hidden")
        state = self.n("reset", control=control, text=text, start=start, length=length)
        if not state["foreground"] or not state["fields"][control]["focused"]:
            # A real click on a validated, unobscured owned control gives focus.
            # No fake-Alt or cross-process AttachThreadInput workaround is used.
            self.f("activate-owned", self.native, self.native_title, "Inline fixture " + control)
            state = self.n("reset", control=control, text=text, start=start, length=length)
            if not state["foreground"] or not state["fields"][control]["focused"]:
                raise RuntimeError("Owned fixture did not obtain input focus")
        return state

    def state(self, control="single"):
        return self.n("state")["fields"][control]

    def metrics(self):
        return self.d("metrics")

    def inline_ready(self, query: str | None = None):
        if not self.d("exists"):
            return False
        if self.args.native_test:
            value = self.metrics()
            if value["inline"]["active"] and value["inline"].get("provider") and value["ready"] and not value["loading"]:
                return value if query is None or value["query"] == query else False
            if not value["inline"]["pending"] and not value["inline"]["active"]:
                raise RuntimeError("Expected inline mode, observed compatibility: " + value["inline"]["status"])
            return False
        dump = self.d("dump")
        ready = "History space" in dump and "ControlType.Edit | Search clipboard history" not in dump
        if query == QUERY:
            ready = ready and PAYLOAD in dump
        elif query and (query.startswith("not-found") or query.startswith("no-result") or query.startswith("邮箱")):
            ready = ready and "No matches in this space" in dump
        return ready

    def side_previews_ready(self, query):
        value = self.metrics()
        return value if all(not side["visible"] or
            (side["ready"] and not side["loading"] and side["query"] == query and side["rows"] <= 4)
            for side in value["side_previews"]) else None

    def manual_history_ready(self):
        if not self.d("exists"):
            return False
        dump = self.d("dump")
        if "History space" not in dump or "ControlType.Edit | Search clipboard history" in dump:
            return False
        if not self.args.native_test:
            return True
        value = self.metrics()
        return value if (value["ready"] and not value["loading"] and value["query"] == ""
                         and not value["inline"]["active"] and not value["inline"]["pending"]
                         and not value["quick_insert"]["has_target"]) else False

    def open_inline(self, query=""):
        deactivations = self.n("state")["deactivations"]
        self.f("hotkey", self.native, self.native_title, "Alt+V")
        value = self.wait(lambda: self.inline_ready(query), "inline popup ready", 12)
        state = self.n("state")
        if not state["foreground"] or state["deactivations"] != deactivations:
            raise RuntimeError("Echo took activation away from the original input")
        dump = self.d("dump")
        if "ControlType.Edit | Search clipboard history" in dump:
            raise RuntimeError("Inline mode still exposes its own search field")
        return value

    def type_query(self, query=QUERY):
        self.f("text", self.native, self.native_title, query)
        return self.wait(lambda: self.inline_ready(query), "actual composer query propagated", 10)

    def confirm_ready(self):
        if self.args.native_test:
            return self.wait(lambda: self.metrics()["inline"].get("readiness", [0] * 8)[7] == 1,
                             "current query result is armed for confirmation", 5)
        return True

    def enter(self, control="single", held=False, expected=None):
        self.confirm_ready()
        self.f("held-enter" if held else "key", self.native, self.native_title, *([] if held else [13]))
        self.wait(lambda: not self.d("exists"), "replacement acknowledged and popup hidden", 8)
        state = self.state(control)
        if state["enter_count"] != 0:
            raise RuntimeError("Enter reached the native input's submit handler")
        if expected is not None and state["text"] != expected:
            raise RuntimeError(f"Wrong replacement: actual={state['text']!r}, expected={expected!r}")
        return state

    def check(self, name: str, function):
        if self.args.only and name not in self.args.only.split(","):
            return
        print("CHECK", name, datetime.now(timezone.utc).isoformat(), flush=True)
        try:
            value = function()
            self.checks.append({"name": name, "status": "PASS", "actual": value})
            print("PASS", name, flush=True)
            atomic(self.evidence / "checks.json", self.checks)
        except NotRun as error:
            self.checks.append({"name": name, "status": "NOT_RUN", "reason": str(error)})
            atomic(self.evidence / "checks.json", self.checks)
            print("NOT_RUN", name, str(error), flush=True)
        except Exception as error:
            self.checks.append({"name": name, "status": "FAIL", "error": str(error)})
            if self.native and self.native.poll() is None:
                try: atomic(self.evidence / ("failure-" + name + ".native.json"), self.n("state"))
                except Exception: pass
            atomic(self.evidence / "checks.json", self.checks)
            try:
                if self.echo and self.echo.poll() is None and self.args.native_test:
                    atomic(self.evidence / ("failure-" + name + ".last-metrics.json"), self.metrics())
                if self.echo and self.echo.poll() is None and self.d("exists"):
                    (self.evidence / ("failure-" + name + ".uia.txt")).write_text(self.d("dump"), encoding="utf-8")
                    if self.args.native_test:
                        atomic(self.evidence / ("failure-" + name + ".metrics.json"), self.metrics())
            except Exception:
                pass
            if self.browser and self.browser.poll() is None:
                try: atomic(self.evidence / ("failure-" + name + ".browser.json"), self.browser_state())
                except Exception: pass
            raise

    def shot(self, name: str):
        if self.args.native_test:
            path = self.evidence / (name + ".png")
            iteration = 1
            while path.exists():
                iteration += 1
                path = self.evidence / (name + f"-{iteration:03d}.png")
            self.d("capture", str(path))

    def start_echo(self):
        if not self.tool("EchoInlineDriver.exe", ["probe",str(self.evidence),"Alt+V"])["available"]:
            raise RuntimeError("Alt+V is owned by another instance; no foreground test was started")
        self.echo = self.launch(self.args.executable, ["--background"], "echo")
        self.env["ECHO_ACCEPTANCE_PID"] = str(self.echo.pid)
        self.record(self.echo, TITLE)
        self.wait(lambda: not self.tool("EchoInlineDriver.exe", ["probe", str(self.evidence), "Alt+V"])["available"], "Alt+V registered")

    def start_processes(self):
        self.start_echo()
        self.native = self.launch(self.args.tools / "EchoInlineFixture.exe", [str(self.evidence), self.native_title], "native-fixture")
        self.record(self.native, self.native_title)
        self.wait(lambda: (self.evidence / "native-ready.json").exists(), "native fixture ready")
        self.wait(lambda: not self.tool("EchoInlineDriver.exe", ["probe", str(self.evidence), "Alt+V"])["available"], "Alt+V registered")

    def application_checks(self):
        """Black-box input in one explicitly registered ordinary application draft.

        This registration is never a cleanup ownership record. Every command
        rechecks HWND, process birth/image, draft marker, focused editor, and
        exact allowed synthetic values. No submission handler is intercepted.
        """
        targets = json.loads(self.args.application_target.read_text(encoding="utf-8-sig"))
        if len(targets) != 1:
            raise RuntimeError("Exactly one dedicated application draft is required")
        target = targets[0]
        if target.get("cleanup_process") is not False or not target.get("forbidden_windows"):
            raise RuntimeError("Shared application cleanup and executing-window exclusion must be explicit")
        atomic(self.evidence / "application-input-targets.json", targets)
        application = SimpleNamespace(pid=target["pid"])
        title = target["title"]
        def command(operation, *arguments):
            return self.f(operation, application, title, *arguments)
        def state():
            return command("application-state")
        initial = state()
        if initial["text"] not in ("", "\n" + target["composer_name"], "\r\n" + target["composer_name"]):
            raise RuntimeError("Actual application must start with an empty synthetic draft")
        atomic(self.evidence / "application-environment.json", {
            "actual_application": True, "external_enter_interceptor": False,
            "forced_accessibility": False, "target": target,
            "application_version": file_version(Path(target["executable"])),
            "application_sha256": hashlib.sha256(Path(target["executable"]).read_bytes()).hexdigest(),
            "input_kind": "SendInput; not physical keyboard",
        })
        self.start_echo()
        def round_trip():
            # No Enter after the popup closes: normal submission restoration is
            # tested in controlled hosts where a submit cannot launch work.
            command("paced-hotkey", "Alt+V", 100)
            self.wait(lambda: self.inline_ready(""), "actual application empty inline scope")
            query = "ec prf text 0013 "
            command("text", "e")
            self.wait(lambda: self.inline_ready("e"), "actual application first character")
            command("text", query[1:])
            self.wait(lambda: self.inline_ready(query), "actual application multiword query")
            if not self.args.native_test:
                self.wait(lambda: self.inline_ready(QUERY), "one actual application candidate")
            command("key", 40)
            command("key", 38)
            self.confirm_ready()
            if state()["text"] != query:
                raise RuntimeError("Actual composer changed before confirmation")
            command("key", 13)
            self.wait(lambda: not self.d("exists"), "actual application completion closed")
            result = state()
            if result["text"] != PAYLOAD:
                raise RuntimeError("Actual application replacement differs; scenario stopped")
            # One native Undo must restore only the query. Cleanup is confined
            # to the exact verified synthetic draft and never writes a task.
            command("hotkey", "Ctrl+Z")
            if state()["text"] != query:
                raise RuntimeError("Actual application Undo did not restore the query")
            command("hotkey", "Ctrl+A")
            command("key", 8)
            if state()["text"] != initial["text"]:
                raise RuntimeError("Actual application draft did not return to empty")
            return {"replacement": result, "undo": "query restored", "draft_still_present": True}
        self.check("actual-application-exact-replacement-and-undo", lambda: self.repeat_case(
            "actual-application", self.args.application_rounds, round_trip))

    def native_checks(self):
        self.start_processes()
        self.check("installed-chinese-ime-confirmation-and-insertion", self.test_installed_ime)
        self.check("startup-manual-history-has-no-local-search", self.test_manual_history)
        self.check("f6-manual-history-preserves-query", lambda:self.test_manual_history(True))
        self.check("fuzzy-words-and-trailing-spaces-keep-actions-stable", self.test_fuzzy_words)
        self.check("inline-query-is-scoped-and-preserved-across-spaces", self.test_scoped_query)
        self.check("transient-root-focus-preserves-original-session", self.test_root_focus)
        self.check("manual-history-row-copy-retains-text", self.test_manual_copy)
        self.check("first-popup-never-activates", self.test_first)
        self.check("delayed-acquisition-keeps-enter-protected", self.test_delayed_acquisition)
        self.check("composer-query-keeps-session-geometry-stable", self.test_height)
        self.check("enter-replaces-query-not-prefix-or-suffix", self.test_exact_replacement)
        self.check("real-top-and-bottom-input-placement", self.test_input_placement)
        self.check("held-enter-never-leaks-submit-after-close", self.test_held)
        self.check("no-results-enter-is-consumed", self.test_empty)
        self.check("fast-new-query-cannot-use-old-result", self.test_fast)
        self.check("pending-query-protects-enter", self.test_pending_query)
        self.check("late-query-response-cannot-replace-current", self.test_late_query)
        self.check("provider-selection-failures-never-replay", self.test_provider_selection_faults)
        self.check("unknown-composition-interface-recovers", self.test_unknown_composition)
        self.check("query-backspace-and-unicode-are-observed", self.test_unicode)
        self.check("preselected-query-replaced-exactly", self.test_selection)
        self.check("hidden-reclaim-preserves-inline-insertion", self.test_hidden_reclaim)
        self.check("selection-refusal-protects-enter-and-recovers", self.test_selection_refusal)
        self.check("native-observation-failure-protects-enter-and-recovers", self.test_read_failure)
        self.check("same-window-other-editor-never-receives-replacement", self.test_other_editor)
        self.check("selection-stage-clipboard-and-focus-races", self.test_selection_races)
        self.check("unknown-paste-outcome-requires-explicit-cancel", self.test_unknown_paste)
        self.check("native-clipboard-contention-and-undo", self.test_clipboard_contention)
        self.check("native-unicode-duplicate-range", self.test_unicode_duplicate_range)
        self.check("native-newline-range-matrix", self.test_newline_ranges)
        self.check("native-nbsp-exact-range", self.test_nbsp_range)
        self.check("native-delayed-readback-uses-request-budget", self.test_delayed_readback)
        self.check("paste-into-query-is-observed", self.test_pasted_query)
        self.check("arrows-move-suggestion-not-original-caret", self.test_arrows)
        self.check("outside-range-movement-keeps-enter-protected", self.test_outside)
        self.check("multiline-native-input-replacement", lambda: self.test_control("multiline"))
        self.check("rich-edit-native-input-replacement", lambda: self.test_control("rich"))
        self.check("unsupported-protected-input-is-explicit-compatibility", self.test_password)
        self.check("f6-preserves-query-and-opens-manual-history", self.test_f6)
        self.check("cancel-and-rearm-preserves-key-lifecycles", self.test_cancel_matrix)
        self.check("streaming-filter-never-clears-the-panel", self.test_streaming_filter)
        if self.args.stress:
            self.check("native-acquisition-delay-stress", lambda: self.repeat_case("delayed-acquisition", 100, self.test_delayed_acquisition))
            self.check("native-cancel-rearm-stress", lambda: self.repeat_case("cancel-rearm", 30, self.test_cancel_matrix))
            self.check("native-filtering-states-stress", lambda: self.repeat_case("filtering-states", 100,
                       lambda: {"filtering": self.test_pending_query(), "empty": self.test_empty(), "suspended": self.test_read_failure()}))
            self.check("native-late-query-stress", lambda: self.repeat_case("late-query", 100, self.test_late_query))
            self.check("native-provider-selection-stress", lambda: self.repeat_case("provider-selection", 30, self.test_provider_selection_faults))
            self.check("native-unknown-composition-stress", lambda: self.repeat_case("unknown-composition", 30, self.test_unknown_composition))
            self.check("native-held-enter-stress", lambda: self.repeat_case("held-enter", 100, self.test_held))
            self.check("native-empty-enter-stress", lambda: self.repeat_case("empty-enter", 100, self.test_empty))
            self.check("native-fast-enter-stress", lambda: self.repeat_case("fast-enter", 100, self.test_fast))
            self.check("native-selection-refusal-stress", lambda: self.repeat_case("selection-refusal", 30, self.test_selection_refusal))
            self.check("native-pasted-query-stress", lambda: self.repeat_case("pasted-query", 30, self.test_pasted_query))
            self.check("native-unknown-paste-stress", lambda: self.repeat_case("unknown-paste", 30, self.test_unknown_paste))
            self.check("native-clipboard-contention-stress", lambda: self.repeat_case("clipboard-contention", 30, self.test_clipboard_contention))
            self.check("native-exact-range-stress", lambda: self.repeat_case("exact-range", 30, self.test_exact_range))
            self.check("native-unicode-range-stress", lambda: self.repeat_case("unicode-range", 20, self.test_unicode_duplicate_range))
            self.check("native-newline-range-stress", lambda: self.repeat_case("newline-range", 20, self.test_newline_ranges))
            self.check("native-nbsp-range-stress", lambda: self.repeat_case("nbsp-range", 20, self.test_nbsp_range))
            self.check("native-delayed-readback-stress", lambda: self.repeat_case("delayed-readback", 30, self.test_delayed_readback))
            self.check("native-read-failure-stress", lambda: self.repeat_case("read-failure", 100, self.test_read_failure))
            self.check("native-other-editor-stress", lambda: self.repeat_case("other-editor", 30, self.test_other_editor))
            self.check("native-selection-race-stress", lambda: self.repeat_case("selection-races", 30, self.test_selection_races))

    def repeat_case(self, name, count, operation):
        results = []
        for index in range(count):
            try:
                value = operation()
            except Exception as error:
                atomic(self.evidence / (name + "-iterations.json"), results)
                raise RuntimeError(f"{name} iteration {index + 1}/{count}: {error}") from error
            results.append({"iteration": index + 1, "status": "PASS", "value": value})
            atomic(self.evidence / (name + "-iterations.json"), results)
            if (index + 1) % 10 == 0:
                print(f"PROGRESS {name} {index + 1}/{count}", flush=True)
        return {"iterations": count, "status": "PASS", "evidence": name + "-iterations.json"}

    def control_request(self, verb, **fields):
        ident = uuid.uuid4().hex
        atomic(self.evidence / "native-control" / "request.json", dict(id=ident,pid=self.echo.pid,verb=verb,**fields))
        def response():
            value=json.loads((self.evidence / "native-control" / "response.json").read_text(encoding="utf-8"))
            if value.get("id") != ident: return None
            if value.get("status") != "PASS": raise RuntimeError(str(value))
            return value["value"]
        return self.wait(response,"native trace " + verb,12)

    def test_installed_ime(self):
        if not self.args.native_test:
            raise NotRun("Composition diagnostics require native-test; physical acceptance is separate")
        self.reset();self.open_inline()
        self.n("ime-chinese",control="single")
        self.f("key",self.native,self.native_title,ord("N"))
        self.f("key",self.native,self.native_title,ord("I"))
        composed=self.wait(lambda:s if (s:=self.state())["ime"]["composition_bytes"]>0 else None,"installed Chinese IME started",8)
        atomic(self.evidence/"foreign-ime-probe.json",self.f("foreign-ime-probe",self.native,self.native_title))
        atomic(self.evidence/"ime-window-metadata.json",self.f("ime-metadata",self.native,self.native_title))
        self.wait(lambda:self.metrics()["inline"]["composing"],"Echo observes real IME composition",8)
        self.shot("installed-ime-composing")
        self.f("key",self.native,self.native_title,13)
        committed=self.wait(lambda:s if (s:=self.state())["ime"]["composition_bytes"]==0 else None,"Enter confirms IME",8)
        if committed["enter_count"]!=0 or not self.d("exists"):
            raise RuntimeError("IME confirmation submitted the input or closed the completion session")
        self.n("ime-english",control="single")
        actual=committed["text"][4:-6]
        self.wait(lambda:self.inline_ready(actual),"committed IME text becomes query",8)
        self.n("selection",control="single",start=4,length=len(actual.encode("utf-16-le"))//2)
        self.type_query("ec prf 0013")
        result=self.enter(expected="pre|"+PAYLOAD+" |post")
        return {"input":"installed Chinese IME driven by synthetic VK keys, not a manual physical-keyboard test", "composition_bytes":composed["ime"]["composition_bytes"],"ime_enter_submit_count":committed["enter_count"],"replacement":result}

    def record_begin(self, name, target, title):
        if not self.args.record_screen:
            return None
        process = self.launch(self.args.tools / "EchoInlineDriver.exe", [
            "record-window", str(self.evidence), str(self.echo.pid), TITLE,
            str(target.pid), title, name], name)
        self.wait(lambda: (self.evidence / name / "ready").exists(), "screen recording ready")
        return (process, name, time.monotonic())

    def record_end(self, recording):
        if recording is None:
            return
        process, name, started = recording
        time.sleep(max(0, 5.2 - (time.monotonic() - started)))
        (self.evidence / name / "stop").write_text("stop", encoding="ascii")
        process.wait(timeout=15)
        if process.returncode:
            raise RuntimeError("Physical screen recording failed: " + name)
        data = json.loads((self.evidence / name / "frames.json").read_text(encoding="utf-8"))
        frames = data["frames"]
        duration = frames[-1]["ms"] - frames[0]["ms"] if len(frames) > 1 else 0
        fps = (len(frames)-1)*1000/duration if duration else 0
        atomic(self.evidence / (name + "-analysis.json"), {
            "source": data["source"], "frames": len(frames), "duration_ms": duration,
            "fps": fps, "visual_review": "NOT_RUN"})
        if duration < 5000 or fps < 30:
            raise RuntimeError("Recording did not reach five seconds at 30 fps: " + name)

    def test_manual_history(self, fallback=False):
        self.reset()
        if fallback:
            self.open_inline(); self.type_query("ec")
        before = self.state()["text"]
        if fallback:
            self.f("key", self.native, self.native_title, 117)
        else:
            subprocess.run([str(self.args.executable), "--history"], env=self.env,
                           stdin=subprocess.DEVNULL, check=True, timeout=8,
                           creationflags=subprocess.CREATE_NO_WINDOW)
        value = self.wait(self.manual_history_ready, "unfiltered manual history ready", 12)
        if self.state()["text"] != before:
            raise RuntimeError("Opening manual history changed the original input")
        if self.args.native_test and value["snapshot_model_count"] == 0:
            raise RuntimeError("Synthetic history unexpectedly has no visible rows")
        self.shot("f6-manual-history" if fallback else "startup-manual-history")
        self.d("close")
        self.wait(lambda: not self.d("exists"), "manual history closed")
        return {"query_preserved": True, "local_search": False, "paste_target": False}

    def test_scoped_query(self):
        if not self.args.native_test:
            raise NotRun("Scoped result identity requires native-test metrics")
        marker = json.loads((self.args.template / "synthetic-fixture.json").read_text(encoding="utf-8-sig"))
        custom = marker.get("dataset") == "S2"
        cases = [(QUERY, 1, 0, 0), ("Echo favorite 007", 0, None, 0)]
        if custom:
            # Long History documents legitimately match these words as fuzzy
            # subsequences. They are not exact-phrase search and need not be empty.
            cases.append(("Memory50 custom space retained original content", None, 0, 1))
        results = []
        for query, history_rows, saved_rows, custom_rows in cases:
            self.reset(); self.open_inline(); self.type_query(query)
            before = self.state()["text"]
            destinations = [(1, history_rows), (2, saved_rows)]
            if custom:
                destinations.append((3, custom_rows))
            destinations.append((1, history_rows))
            for step, (space, rows) in enumerate(destinations):
                value = self.wait(
                    lambda: m if (m := self.inline_ready(query)) and m["space"] == space
                    and m["phase"] == "Idle" and m["requested"] == str(space)
                    and m["presented"] == str(space) and m["interaction"] == str(space)
                    and (m["snapshot_model_count"] == rows if rows is not None
                         else m["snapshot_model_count"] > 0) else None,
                    "query result in requested space")
                if self.state()["text"] != before or not self.n("state")["foreground"]:
                    raise RuntimeError("Space switching changed the original input or focus")
                if self.args.renderer == "software":
                    value = self.wait(lambda: self.side_previews_ready(query), "side previews use the original input query")
                    expected = {1: history_rows, 2: saved_rows, 3: custom_rows, 4: 0}
                    for side in value["side_previews"]:
                        if not side["visible"]:
                            continue
                        count = expected.get(int(side["space"]))
                        if (count is not None and side["rows"] != min(count, 4)) or (count is None and side["rows"] == 0):
                            raise RuntimeError("Side preview did not filter its corresponding space: " + str(side))
                results.append({"query": query, "space": space, "rows": value["snapshot_model_count"]})
                if step < len(destinations) - 1:
                    target = destinations[step + 1][0]
                    # Fixtures may have additional metadata-only custom spaces.
                    # Walk the actual deck rather than assume Favorites wraps to History.
                    for _ in range(16):
                        self.f("key", self.native, self.native_title, 9)
                        current = self.wait(lambda: m if (m := self.inline_ready(query))
                            and m["phase"] == "Idle" else None, "next scoped space")
                        if current["space"] == target:
                            break
                    else:
                        raise RuntimeError("Requested space was absent from the navigation deck")
            self.f("key", self.native, self.native_title, 27)
            self.wait(lambda: not self.d("exists"), "scoped query canceled")
        return {"endpoint": "original native input and guarded Tab", "results": results}

    def test_fuzzy_words(self):
        if not self.args.native_test:
            raise NotRun("Detailed glyph/action trace requires native-test")
        self.reset(); self.open_inline()
        self.type_query("ec prf 0013")
        self.confirm_ready()
        before=self.metrics()
        if before["snapshot_model_count"]!=1 or before.get("highlighted_rows",0)!=1:
            raise RuntimeError("Multiword subsequence query did not produce one highlighted match")
        self.shot("fuzzy-multiword")
        pixels=self.launch(self.args.tools/"EchoInlineDriver.exe",["sample-actions",str(self.evidence),str(self.echo.pid),TITLE,str(self.native.pid),self.native_title],"action-pixels")
        self.wait(lambda:(self.evidence/"actions.ready").exists(),"action pixel sampler ready")
        self.control_request("trace_begin",file="action-trace")
        progress=[]
        try:
            for part in (" "," ","r"," ","e"):
                self.f("text",self.native,self.native_title,part)
                raw=self.state()["text"][4:-6]
                m=self.wait(lambda:self.inline_ready(raw),"space-delimited fuzzy query")
                self.confirm_ready()
                if m["snapshot_model_count"]!=1 or m["selection"]!=before["selection"]:
                    raise RuntimeError("Refining query lost the existing matching item")
                dump=self.d("dump")
                for line in dump.splitlines():
                    if " | Copy item | " in line or " | Insert item | " in line:
                        if "enabled=False" in line: raise RuntimeError("Action icon appearance was disabled while refining")
                progress.append({"query":raw,"rows":m["snapshot_model_count"],"highlighted":m.get("highlighted_rows")})
                time.sleep(.10)
        finally:
            self.control_request("trace_end")
            (self.evidence/"actions.stop").write_text("stop",encoding="ascii")
            pixels.wait(timeout=15)
        sample=json.loads((self.evidence/"action-pixels.log").read_text(encoding="utf-8-sig"))
        if pixels.returncode!=0 or sample.get("status")!="PASS": raise RuntimeError("Action pixel capture failed")
        physical=sample["value"]["frames"]
        if len(physical)<10: raise RuntimeError("Insufficient action pixel samples")
        base=physical[0]["colors"]
        flicker=[f for f in physical if sum(max(abs(((c>>k)&255)-((d>>k)&255)) for k in (0,8,16))>15 for c,d in zip(f["colors"],base))>len(base)*.12]
        atomic(self.evidence/"action-pixel-analysis.json",{"frames":len(physical),"changed_frames":len(flicker)})
        if flicker: raise RuntimeError("Copy action pixels faded, blinked or moved during the same-item query")
        frames=json.loads((self.evidence/"action-trace/frames.json").read_text(encoding="utf-8"))
        if len(frames)<12: raise RuntimeError("Insufficient action-state samples")
        if any(f["stale"] or not f["visible"] or f["rows"]!=1 or f["selected_rows"]!=1 or f["busy"] for f in frames):
            raise RuntimeError("Pending fuzzy queries cleared selection, buttons or rows")
        self.shot("fuzzy-refined")
        final=self.enter(expected="pre|"+PAYLOAD+" |post")
        return {"frames":len(frames),"progress":progress,"replacement":final}

    def test_empty_rich(self, decorated=False):
        self.browser_reset("ai")
        reset = ("Reset leaf decorated empty composer" if decorated == "leaf" else
                 "Reset decorated empty rich composer" if decorated else "Reset empty rich composer")
        self.f("invoke-control",self.browser,self.browser_title,reset)
        self.wait(lambda:self.browser_state()["ai"]["text"]=="","empty paragraph reset")
        if decorated == "leaf" and self.args.native_test:
            self.inspect_browser_ranges("leaf-decoration-empty-ranges")
        self.browser_open()
        if decorated:
            self.browser_query("x")
            self.f("key", self.browser, self.browser_title, 8)
            self.wait(lambda: self.browser_state()["ai"]["text"] == "" and self.inline_ready(""),
                      "deleting all text restores the decorated empty query")
        typed=""
        progress=[]
        for part in ("e","c"," ","p","rf"," ","0013"):
            self.f("text",self.browser,self.browser_title,part);typed+=part
            def current():
                m=self.inline_ready()
                if not self.args.native_test:
                    return m and self.browser_state()["ai"]["text"].replace("\u00a0", " ") == typed
                return m if m and m["query"].replace("\u00a0"," ")==typed else None
            m=self.wait(current,"empty composer first-key and multiword continuity",8)
            progress.append({"query":typed,"rows":m["snapshot_model_count"] if self.args.native_test else None})
            if not self.browser_state()["ai"]["focused"]: raise RuntimeError("Typing focus left the composer")
        self.confirm_ready()
        if self.args.native_test:
            if self.metrics()["snapshot_model_count"]!=1 or self.metrics().get("highlighted_rows",0)!=1:
                raise RuntimeError("Empty composer did not narrow and highlight a fuzzy match")
        else:
            self.wait(lambda: ("1 match" in self.d("dump")) and PAYLOAD in self.d("dump"), "one visible fuzzy result")
        self.shot("empty-rich-fuzzy")
        self.f("key",self.browser,self.browser_title,13)
        self.wait(lambda:not self.d("exists"),"empty composer replacement acknowledged",8)
        state=self.browser_state()["ai"]
        if state["text"]!=PAYLOAD or state["submitted"]!=0:
            raise RuntimeError("First-character recovery replaced or submitted the wrong content")
        return {"progress":progress,"state":state}

    def test_actual_placeholder_label(self):
        self.browser_reset("ai")
        self.f("invoke-control", self.browser, self.browser_title, "Reset actual label text")
        label = "Inline rich AI composer"
        self.wait(lambda: self.browser_state()["ai"]["text"] == label, "literal label fixture")
        self.browser_open()
        self.browser_query()
        return self.browser_enter("ai", expected=PAYLOAD + label)

    def inspect_browser_ranges(self, name):
        raw = self.f("read-control", self.browser, self.browser_title, "Inline rich AI composer")
        user = ctypes.WinDLL("user32")
        user.GetForegroundWindow.restype = ctypes.c_void_p
        user.GetWindowThreadProcessId.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
        window = user.GetForegroundWindow()
        owner = ctypes.c_uint32()
        user.GetWindowThreadProcessId(window, ctypes.byref(owner))
        if owner.value != self.browser.pid:
            raise RuntimeError("Read-only range probe target lost foreground")
        result = subprocess.run([str(self.root / "target/debug/examples/inline_target_probe.exe"), str(window), raw],
            env=self.env, capture_output=True, text=True, encoding="utf-8", timeout=8,
            creationflags=subprocess.CREATE_NO_WINDOW)
        observation = json.loads(result.stdout.lstrip("\ufeff"))
        atomic(self.evidence / (name + ".json"), observation)
        if result.returncode:
            raise RuntimeError("Read-only range probe failed")
        return observation

    def test_leaf_decoration_literal(self):
        if not self.args.native_test:
            raise NotRun("Range normalization diagnostics require native-test")
        observations = []
        for label, name in (("Reset leaf decoration with literal text", "leaf-decoration-literal-ranges"),
                            ("Reset direct literal placeholder class", "direct-literal-placeholder-ranges")):
            self.browser_reset("ai")
            self.f("invoke-control", self.browser, self.browser_title, label)
            self.wait(lambda: self.browser_state()["ai"]["text"] == "Ask fixture", "literal text fixture")
            observation = self.inspect_browser_ranges(name)
            if observation["range"]["empty_decorated"] or observation["range"]["normalized_units"] == 0:
                raise RuntimeError("A real text node was treated as an empty decoration")
            if self.browser_state()["ai"]["text"] != "Ask fixture":
                raise RuntimeError("Read-only normalization probe changed literal text")
            observations.append(observation)
        return observations

    def test_streaming_filter(self):
        if not self.args.native_test: raise NotRun("Internal frame trace requires native-test")
        self.reset(); self.open_inline()
        initial=self.metrics()
        recording=self.record_begin("inline-screen",self.native,self.native_title)
        pixels=self.launch(self.args.tools/"EchoInlineDriver.exe",["sample-headers",str(self.evidence),str(self.echo.pid),TITLE,str(self.native.pid),self.native_title],"header-samples")
        self.wait(lambda:(self.evidence/"pixels.ready").exists(),"physical header sampler ready")
        self.control_request("trace_begin",file="typing-trace")
        progress=[]
        try:
            typed=""
            for part in ("e","cho","-perf","-text","-00","13"):
                self.f("text",self.native,self.native_title,part);typed+=part
                m=self.wait(lambda:self.inline_ready(typed),"incremental query " + typed,8)
                progress.append({"query":typed,"rows":m["snapshot_model_count"],"height":m["panel"][3]})
                time.sleep(.09)
            self.confirm_ready()
            time.sleep(.15)
        finally:
            self.control_request("trace_end")
            (self.evidence/"pixels.stop").write_text("stop",encoding="ascii")
            pixels.wait(timeout=15)
            self.record_end(recording)
        samples=json.loads((self.evidence/"header-samples.log").read_text(encoding="utf-8-sig"))
        if pixels.returncode or samples.get("status") != "PASS": raise RuntimeError("Physical header sampling failed: "+str(samples))
        physical=samples["value"]["frames"]
        if len(physical)<20: raise RuntimeError("Insufficient physical display samples")
        baseline=physical[0]["colors"]
        def differs(c,d): return max(abs(((c>>i)&255)-((d>>i)&255)) for i in (0,8,16))>3
        flashes=[f for f in physical if sum(differs(c,d) for c,d in zip(f["colors"],baseline))>=7]
        atomic(self.evidence/"physical-header-analysis.json",{"frames":len(physical),"changed_header_frames":len(flashes)})
        if flashes: raise RuntimeError("Opaque header disappeared or flashed on the physical display")
        frames=json.loads((self.evidence/"typing-trace/frames.json").read_text(encoding="utf-8"))
        if len(frames)<12: raise RuntimeError("Insufficient rendering samples")
        if any(not f["visible"] or f["stale"] or f["rows"]==0 or f["navigation_busy"] for f in frames):
            raise RuntimeError("A query hid/blanked the existing panel")
        if any("capture_error" in f for f in frames): raise RuntimeError("Renderer snapshot failed")
        final=self.metrics()
        if final["snapshot_model_count"] != 1 or abs(final["panel"][3] - initial["panel"][3]) > 2:
            raise RuntimeError("Filtering did not retain the latched session height and one exact result")
        if self.state()["text"] != "pre|"+QUERY+" |post": raise RuntimeError("Typing left the original composer")
        self.shot("streaming-final")
        self.f("key",self.native,self.native_title,27)
        self.wait(lambda:not self.d("exists"),"streaming test cancelled")
        return {"frames":len(frames),"physical_display_samples":len(physical),"renderer_snapshots":sum("png" in f for f in frames),"blank_frames":0,"progress":progress}

    def test_first(self):
        state=self.reset(); atomic(self.evidence / "native-input-capabilities.json", {"ime":state["fields"]["single"]["ime"],"patterns":self.f("patterns",self.native,self.native_title,"Inline fixture single")}); m = self.open_inline(); self.shot("01-inline-empty-query")
        return {"input_foreground": True, "deactivations": self.n("state")["deactivations"], "metrics": m if self.args.native_test else None}

    def test_delayed_acquisition(self):
        if not self.args.native_test:
            raise NotRun("Acquisition ordering requires diagnostic hook timestamps")
        results = []
        for delay in (250, 450):
            self.reset()
            before = self.metrics()["provider_faults"]["acquisitions"]
            self.control_request("provider_fault", key=delay)
            trace_name = "acquisition-" + uuid.uuid4().hex
            self.control_request("trace_begin", file=trace_name)
            try:
                self.f("hotkey-enter", self.native, self.native_title, "Alt+V", 50)
                self.wait(lambda: self.inline_ready(""), "delayed initial target inspection")
                after = self.state(); metrics = self.metrics()
                if metrics["provider_faults"]["acquisitions"] <= before or after["enter_count"] or after["text"] != "pre| |post":
                    raise RuntimeError("Initial read delay was absent or the early Enter escaped protection")
                trace = metrics["inline_trace"]
                session = metrics["inline"]["readiness"][0]
                events = [entry for entry in trace if entry["session"] == session]
                armed = next((entry for entry in events if entry["kind"] == "armed"), None)
                entered = next((entry for entry in events if entry["kind"] == "enter-decision"), None)
                fault_start = next((entry for entry in events if entry["kind"] == "acquisition-fault-start"), None)
                fault_end = next((entry for entry in events if entry["kind"] == "acquisition-fault-end"), None)
                if not all((armed,entered,fault_start,fault_end)) or not armed["us"] <= fault_start["us"] < entered["us"] < fault_end["us"]:
                    raise RuntimeError("Early confirmation did not occur inside the armed acquisition fault")
                self.f("key", self.native, self.native_title, 27)
                self.wait(lambda: not self.d("exists"), "delayed acquisition cancelled")
                results.append({"delay_ms": delay, "after": after, "trace": events,
                                "display_trace": trace_name, "unavailable": metrics["inline"]["unavailable"]})
            finally:
                self.control_request("trace_end")
                self.control_request("provider_fault")
        return results

    def test_height(self):
        self.reset(); self.open_inline()
        before = self.f("geometry", self.echo, TITLE)
        self.type_query(); self.shot("02-inline-filtered")
        after = self.f("geometry", self.echo, TITLE)
        if any(abs(a-b)>1 for a,b in zip(before["window"], after["window"])):
            raise RuntimeError("Filtering changed the current session's anchored geometry")
        if self.args.native_test and self.metrics()["snapshot_model_count"] != 1:
            raise RuntimeError("Filtering did not narrow the result model")
        if self.state()["text"] != "pre|" + QUERY + " |post":
            raise RuntimeError("Typing did not stay in the composer")
        return {"before": before, "after": after, "query": QUERY}

    def test_root_focus(self):
        if not self.args.native_test:
            raise NotRun("Session identity assertions require native-test")
        self.reset(); self.open_inline(); self.type_query()
        session = self.metrics()["inline"]["readiness"][0]
        for _ in range(4):
            self.n("root-focus-roundtrip")
            self.wait(lambda: self.state()["focused"] and self.inline_ready(QUERY), "original editor revalidated after root transition")
            self.confirm_ready()
            if self.metrics()["inline"]["readiness"][0] != session:
                raise RuntimeError("Root focus transition cancelled or replaced the original session")
            if self.state()["text"] != "pre|" + QUERY + " |post" or self.state()["enter_count"] != 0:
                raise RuntimeError("Root focus transition changed the host input")
        return {"transitions":4, "replacement":self.enter(expected="pre|" + PAYLOAD + " |post")}

    def test_manual_copy(self):
        self.reset(); self.open_inline()
        self.f("key", self.native, self.native_title, 117)
        self.wait(self.manual_history_ready, "manual-copy history ready")
        expected = "SELECT id, updated_at\nFROM clipboard_entries\nORDER BY updated_at DESC;\n-- fixture 0199"
        marker = json.loads((self.args.template / "synthetic-fixture.json").read_text(encoding="utf-8-sig"))
        if marker.get("dataset") in ("T", "M"):
            expected = "echo-perf-text-1999 — Reusable content, available when you need it."
        self.f("activate-owned", self.echo, TITLE, expected)
        self.wait(lambda: self.n("clipboard-matches", expected=expected)["matches"], "original multiline text copied")
        state = self.state()
        if state["text"] != "pre| |post" or state["enter_count"]:
            raise RuntimeError("Manual copying inserted or submitted into the original input")
        return {"clipboard_matches_original": True, "host_input_unchanged": True}

    def test_exact_replacement(self):
        self.reset(); self.open_inline(); self.type_query()
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_input_placement(self):
        evidence=[]
        for lower in (False,True):
            self.reset()
            g=self.f("geometry",self.native,self.native_title)
            work=g["work"]; caret=g["caret"]
            if not caret: raise RuntimeError("Owned native caret unavailable")
            desired=work[3]-140 if lower else work[1]+280
            self.f("move",self.native,self.native_title,g["window"][0],g["window"][1]+desired-caret[1])
            self.n("reset",control="single",text="pre| |post",start=4,length=0)
            g=self.f("geometry",self.native,self.native_title);caret=g["caret"]
            self.open_inline()
            scale=g.get("monitor_dpi",96)/96.0
            card=self.wait(lambda: c if (c:=self.f("card",self.echo,TITLE,"History space"))[3]-c[1]>=480*scale else None,
                           "full-size suggestions use available side",8)
            gap=caret[1]-card[3] if lower else card[1]-caret[3]
            if abs(gap-8*scale)>5:
                raise RuntimeError("Popup detached from measured caret: " + repr((caret,card,gap)))
            if not self.n("state")["foreground"]:
                raise RuntimeError("Popup took input focus")
            evidence.append({"lower":lower,"caret":caret,"card":card,"gap":gap})
            self.shot("placement-lower" if lower else "placement-upper")
            self.type_query()
            smaller=self.f("card",self.echo,TITLE,"History space")
            edge=(3 if lower else 1)
            if abs(smaller[edge]-card[edge])>5:
                raise RuntimeError("Shrinking results moved the anchored edge")
            self.f("key",self.native,self.native_title,27)
            self.wait(lambda:not self.d("exists"),"placement case dismissed")
        self.f("move",self.native,self.native_title,480,300)
        return evidence

    def test_held(self):
        self.reset(); self.open_inline(); self.type_query()
        protected = self.enter(held=True, expected="pre|" + PAYLOAD + " |post")
        # Normal host confirmation is tested only in the isolated counted Edit.
        # It is never attempted against a real chat/task composer.
        self.f("key", self.native, self.native_title, 13)
        restored = self.wait(lambda: s if (s := self.state())["enter_count"] == 1 else None,
                             "normal host Enter restored after the consumed key-up")
        if restored["text"] != protected["text"]:
            raise RuntimeError("Normal confirmation changed the retained replacement")
        return {"protected": protected, "normal_after_close": restored["enter_count"]}

    def test_empty(self):
        self.reset(); self.open_inline(); self.type_query("not-found-echo-987654321")
        self.f("key", self.native, self.native_title, 13)
        time.sleep(0.12)
        state = self.state()
        if state["enter_count"] != 0 or not self.d("exists") or state["text"] != "pre|not-found-echo-987654321 |post":
            raise RuntimeError("Empty results leaked Enter or changed text")
        self.shot("03-inline-no-results")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "Esc dismissed")
        return state

    def test_fast(self):
        self.reset(); self.open_inline()
        self.f("text-enter", self.native, self.native_title, "not-found-fast-query")
        time.sleep(0.2)
        state = self.state()
        if state["enter_count"] or state["text"] != "pre|not-found-fast-query |post":
            raise RuntimeError("An old result or Enter was delivered during a fast query change")
        return state

    def test_pending_query(self):
        if not self.args.native_test:
            raise NotRun("Deterministic query delay requires the diagnostic candidate")
        self.reset(); self.open_inline(); self.type_query()
        initial = self.metrics()
        self.control_request("search_fault", key=1000)
        self.f("text", self.native, self.native_title, "z")
        pending = self.wait(lambda: m if (m := self.metrics())["loading"]
                            and m["search_faults"]["delayed"] > initial["search_faults"]["delayed"] else None,
                            "worker query remains deliberately pending")
        if pending["snapshot_model_count"] != initial["snapshot_model_count"]:
            raise RuntimeError("Pending query discarded the old complete picture")
        self.f("key", self.native, self.native_title, 13)
        protected = self.state()
        if protected["enter_count"] or protected["text"] != "pre|" + QUERY + "z |post":
            raise RuntimeError("Pending query used the old row or submitted Enter")
        self.wait(lambda: self.inline_ready(QUERY + "z"), "latest query finishes after the injected delay")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "pending-query scenario cancelled")
        return {"pending_rows": pending["snapshot_model_count"], "protected": protected}

    def test_late_query(self):
        if not self.args.native_test:
            raise NotRun("Out-of-order worker responses require the diagnostic candidate")
        self.reset(); self.open_inline()
        initial = self.metrics()["search_faults"]
        self.control_request("search_fault", ctrl=True)
        query = "ec prf text 0013"
        self.f("text", self.native, self.native_title, query[:-1])
        self.wait(lambda: self.metrics()["search_faults"]["held"] > initial["held"], "old result held after matching")
        self.f("text-enter", self.native, self.native_title, query[-1])
        self.wait(lambda: self.metrics()["search_faults"]["late"] > initial["late"], "older result delivered after the newer result")
        self.wait(lambda: self.inline_ready(query), "latest query remains presented after reversed results")
        state = self.state(); metrics = self.metrics()
        if state["enter_count"] or state["text"] != "pre|" + query + " |post" or metrics["snapshot_model_count"] != 1:
            raise RuntimeError("Late response changed text, submitted or displaced the latest result")
        return {"protected": state, "faults": metrics["search_faults"],
                "replacement": self.enter(expected="pre|" + PAYLOAD + " |post")}

    def test_unicode(self):
        self.reset(); self.open_inline(); self.type_query("邮箱🙂")
        self.f("key", self.native, self.native_title, 8)
        def coherent_backspace():
            actual=self.state()["text"][4:-6]
            return (actual if len(actual.encode("utf-16-le")) < 8 and self.inline_ready(actual) else None)
        actual=self.wait(coherent_backspace, "Unicode backspace matches actual provider text")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "Unicode query cancelled")
        if self.state()["text"] != "pre|" + actual + " |post":
            raise RuntimeError("Esc removed typed query")
        return {"actual_query": actual, "submit_count": self.state()["enter_count"]}

    def test_selection(self):
        self.reset(text="pre|" + QUERY + " |post", start=4, length=len(QUERY))
        self.open_inline("")
        if self.args.native_test:
            self.wait(lambda: self.side_previews_ready(""), "side cards ignore preexisting selection")
        original = self.state()
        if original["text"] != "pre|" + QUERY + " |post":
            raise RuntimeError("Opening Echo changed the preexisting selection text")
        direct = self.enter(expected="pre|" + PAYLOAD.replace("0013", "1999") + " |post")
        self.reset(text="pre|old selected text |post", start=4, length=len("old selected text"))
        self.open_inline("")
        self.type_query()
        if self.args.native_test:
            self.wait(lambda: self.side_previews_ready(QUERY), "new input filters both side cards")
        return {"direct": direct, "new_query": self.enter(expected="pre|" + PAYLOAD + " |post")}

    def test_hidden_reclaim(self):
        self.reset()
        self.open_inline()
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "inline popup hidden before reclamation")
        def reclaimed():
            events = [json.loads(line) for line in
                (self.evidence / f"lifecycle-{self.echo.pid}.jsonl").read_text(encoding="utf-8-sig").splitlines()]
            hidden = [e for e in events if e["state"] == "hidden_warm"][-1]
            matches = [e for e in events if e["state"] == "hidden_reclaimed"
                and e["details"]["epoch"] == hidden["details"]["epoch"]
                and e["details"]["hidden_generation"] == hidden["details"]["hidden_generation"]]
            if not matches:
                return None
            ack = matches[-1]
            elapsed = (int(ack["utc_ns"]) - int(hidden["utc_ns"])) / 1e9
            if not 30 <= elapsed <= 35 or not ack["details"]["worker_cache_acknowledged"]:
                raise RuntimeError("Hidden reclamation contract was not satisfied")
            return {"seconds": elapsed, "acknowledgement": ack}
        recovery = self.wait(reclaimed, "actual hidden reclamation acknowledgement", 36)
        self.open_inline()
        self.type_query()
        return {"recovery": recovery, "insertion": self.enter(expected="pre|" + PAYLOAD + " |post")}

    def test_pasted_query(self):
        self.reset()
        # The user copies the query before invoking Echo. Do not conflate this
        # scenario with an immediate OLE writer/reader contention setup; that
        # failure remains separately recorded and the mutation fault is explicit.
        self.f("clipboard-text", self.native, self.native_title, QUERY)
        self.open_inline()
        self.f("hotkey", self.native, self.native_title, "Ctrl+V")
        self.wait(lambda: self.inline_ready(QUERY), "pasted query filtered")
        query_state = self.state()
        return {"query_paste": query_state,
                "replacement": self.enter(expected="pre|" + PAYLOAD + " |post")}

    def test_exact_range(self):
        self.reset(); self.open_inline(); self.type_query()
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_unicode_duplicate_range(self):
        prefix = QUERY + " / 👨‍👩‍👧‍👦 Cafe\u0301 中文|"
        suffix = "|中文 e\u0301 👨‍👩‍👧‍👦 / " + QUERY
        self.reset(text=prefix + QUERY + suffix, start=len(prefix.encode("utf-16-le")) // 2,
                   length=len(QUERY.encode("utf-16-le")) // 2)
        self.open_inline(QUERY)
        return self.enter(expected=prefix + PAYLOAD + suffix)

    def test_newline_ranges(self):
        results = []
        for prefix, query, suffix in (
            ("before\r\n", QUERY, "\r\nafter"),
            ("before\r\n\r\n\r\n", QUERY, "\r\n\r\nafter"),
            ("before|", "\r\n" + QUERY + "\r\n", "|after"),
        ):
            self.reset("multiline", text=prefix + query + suffix,
                       start=len(prefix.encode("utf-16-le")) // 2,
                       length=len(query.encode("utf-16-le")) // 2)
            self.open_inline(query)
            results.append(self.enter("multiline", expected=prefix + PAYLOAD + suffix))
        return results

    def test_nbsp_range(self):
        self.reset(); self.open_inline(); self.type_query("ec\u00a0prf  0013 ")
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_arrows(self):
        self.reset(); self.open_inline(); before = self.state()
        m1 = self.metrics() if self.args.native_test else None
        self.f("key", self.native, self.native_title, 40)
        time.sleep(0.1); after = self.state()
        if (before["start"], before["length"], before["text"]) != (after["start"], after["length"], after["text"]):
            raise RuntimeError("Arrow key changed the original input instead of the suggestion")
        if self.args.native_test and m1["selection"] == self.metrics()["selection"]:
            raise RuntimeError("Arrow did not move suggestion highlight")
        self.f("hotkey", self.native, self.native_title, "Alt+V")
        self.wait(lambda: not self.d("exists"), "second Alt+V dismissed")
        return {"caret_preserved": True, "second_hotkey_dismissed": True}

    def test_outside(self):
        self.reset(); self.open_inline()
        self.f("key", self.native, self.native_title, 37)
        self.wait(lambda: self.metrics()["inline"]["readiness"][7] == 0, "caret left query; execution suspended")
        before = self.state()
        self.f("key", self.native, self.native_title, 13)
        after = self.state()
        if not self.d("exists") or after["enter_count"] or after["text"] != before["text"]:
            raise RuntimeError("Suspended query leaked Enter or changed text")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda:not self.d("exists"), "explicit cancellation")
        return after

    def test_selection_refusal(self):
        self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
        before = self.state()
        self.n("selection-policy", reject=True)
        try:
            for _ in range(2):
                self.f("key", self.native, self.native_title, 13)
                self.wait(lambda: "exact selection" in self.metrics()["inline"]["status"], "selection rejection reported")
                after = self.state()
                if after["text"] != before["text"] or after["enter_count"] != 0 or not self.d("exists"):
                    raise RuntimeError("Refused selection changed text, leaked Enter, or removed protection")
                self.confirm_ready()
        finally:
            self.n("selection-policy", reject=False)
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_read_failure(self):
        if not self.args.native_test:
            raise NotRun("Suspended-state fault tracing requires the diagnostic candidate")
        self.reset(); self.open_inline()
        refused_before = self.state()["refused_read_count"]
        self.n("read-refusal-policy", enabled=True)
        try:
            self.f("text", self.native, self.native_title, QUERY)
            self.wait(lambda: self.metrics()["inline"]["suspended"], "failed external observation pauses the session")
            self.f("key", self.native, self.native_title, 13)
            before = self.state()
            if before["enter_count"] or before["refused_read_count"] <= refused_before or before["text"] != "pre|" + QUERY + " |post" or not self.d("exists"):
                raise RuntimeError("Failed observation lost text, confirmation protection or popup")
        finally:
            self.n("read-refusal-policy", enabled=False)
        self.f("text", self.native, self.native_title, " ")
        self.wait(lambda: self.inline_ready(QUERY + " ") and not self.metrics()["inline"]["suspended"],
                  "new input resumes the original query range")
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_provider_selection_faults(self):
        if not self.args.native_test:
            raise NotRun("Provider faults require the diagnostic candidate")
        results = []
        for name, flags in (("E_FAIL", {"file":"refuse-selection"}), ("S_OK-no-change", {"shift":True}), ("late-readback", {"ctrl":True})):
            self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
            before = self.state(); count = self.metrics()["provider_faults"]["selections"]
            self.control_request("provider_fault", **flags)
            try:
                self.f("key", self.native, self.native_title, 13)
                self.wait(lambda: self.metrics()["provider_faults"]["selections"] > count,
                          "selection fault reached the actual adapter boundary")
                failure = self.wait(lambda: v if (v:=self.metrics())["error"] else None, "selection failure reported")
                time.sleep(1.1)
                after = self.state()
                if after["text"] != before["text"] or after["enter_count"] or after["range_replace_attempts"] or after["paste_attempts"] or not self.d("exists"):
                    raise RuntimeError("Failed/late selection changed text, submitted, replayed, or dropped protection")
                if name == "late-readback" and ("SelectionUnconfirmed" not in failure["status"] or failure["inline_safety"][1]):
                    raise RuntimeError("Selection-only timeout was misreported as an unknown text replacement")
                self.n("selection", control="single", start=4+len(QUERY), length=0)
                self.f("text", self.native, self.native_title, " ")
                self.wait(lambda: self.state()["text"] == before["text"][:4+len(QUERY)]+" "+before["text"][4+len(QUERY):], "editing remains possible after selection failure")
                self.f("key", self.native, self.native_title, 117)
                self.wait(self.manual_history_ready, "F6 after rejected selection")
                self.d("close"); self.wait(lambda:not self.d("exists"), "fallback closed")
                results.append({"fault":name,"before":before,"after":after,"status":failure["status"],"replayed":False})
            finally:
                self.control_request("provider_fault")
        return results

    def test_unknown_composition(self):
        if not self.args.native_test:
            raise NotRun("Composition interface fault requires the diagnostic candidate")
        results=[]
        for cancel in (27,117):
            self.reset(); self.control_request("provider_fault", paused=True)
            try:
                self.open_inline(); self.type_query()
                unknown=self.wait(lambda:v if (v:=self.metrics())["inline"]["suspended"] and v["inline"]["readiness"][7]==0 else None,"unqueryable composition is Unknown")
                self.f("key",self.native,self.native_title,13)
                self.f("text",self.native,self.native_title," ")
                after=self.state()
                if after["text"]!="pre|"+QUERY+"  |post" or after["enter_count"] or after["range_replace_attempts"] or not self.d("exists"):
                    raise RuntimeError("Unknown composition leaked confirmation or prevented ordinary text input")
                self.f("key",self.native,self.native_title,cancel)
                if cancel==117:
                    self.wait(self.manual_history_ready,"Unknown can use F6")
                    self.d("close")
                self.wait(lambda:not self.d("exists"),"Unknown can explicitly exit")
                results.append({"cancel":cancel,"unknown_readiness":unknown["inline"]["readiness"],"after":after})
            finally:
                self.control_request("provider_fault")
            self.reset(); self.open_inline(); self.type_query()
            results[-1]["new_session"]=self.enter(expected="pre|"+PAYLOAD+" |post")
        return results

    def test_other_editor(self):
        if not self.args.native_test:
            raise NotRun("The deliberately late focus-notification scenario requires diagnostic fault injection")
        self.reset("multiline")
        self.reset(); self.open_inline(); self.type_query()
        original = self.state()
        self.control_request("pause_inline_window_events", paused=True)
        try:
            changed = self.n("focus", control="multiline")
            before = changed["fields"]["multiline"]["text"]
            if not self.metrics()["inline"]["active"] or not self.d("exists"):
                raise RuntimeError("Late-notification fault did not retain the old visible lease")
            self.f("key", self.native, self.native_title, 13)
            self.wait(lambda: not self.d("exists"), "same-window editor switch cancels the old session")
            after = self.n("state")
        finally:
            self.control_request("pause_inline_window_events", paused=False)
        if after["fields"]["single"]["text"] != original["text"] or after["fields"]["multiline"]["text"] != before:
            raise RuntimeError("Another editor received an old replacement")
        if after["fields"]["multiline"]["enter_count"] or after["fields"]["multiline"]["range_replace_attempts"]:
            raise RuntimeError("The stale lease leaked Enter or wrote to the other editor")
        return after

    def test_selection_races(self):
        results = []
        for fault in ("clipboard", "focus"):
            self.reset("multiline"); self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
            before = self.n("state")
            self.n("selection-fault-policy", fault=fault)
            try:
                self.f("key", self.native, self.native_title, 13)
                self.wait(lambda: self.state()["selection_fault_count"] == 1, "race injected after exact selection")
                time.sleep(.35)
                after = self.n("state")
                field = after["fields"]["single"]
                if field["selection_fault_error"]:
                    raise RuntimeError("Selection fault could not be injected: " + field["selection_fault_error"])
                for key in ("single", "multiline"):
                    if after["fields"][key]["text"] != before["fields"][key]["text"] or after["fields"][key]["enter_count"]:
                        raise RuntimeError("Selection race changed content or leaked confirmation")
                if field["range_replace_attempts"] or field["paste_attempts"]:
                    raise RuntimeError("Stale clipboard/focus passed the native write boundary")
                results.append({"fault": fault, "state": after})
            finally:
                self.n("selection-fault-policy", fault="")
            if self.d("exists"):
                self.f("key", self.native, self.native_title, 27)
                self.wait(lambda: not self.d("exists"), "race scenario explicitly cancelled")
        return results

    def test_unknown_paste(self):
        self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
        self.n("paste-reply-policy", delay_ms=250)
        try:
            self.f("key", self.native, self.native_title, 13)
            if self.args.native_test:
                self.wait(lambda: (m["inline"]["suspended"] and m["inline_safety"][1]) if (m := self.metrics()) else False,
                          "unknown paste outcome is visibly suspended")
            else:
                self.wait(lambda: "Check the input" in self.d("dump") or "Replacement outcome is unknown" in self.d("dump"),
                          "production unknown-outcome notice")
            before = self.state()
            self.n("selection", control="single", start=0, length=0)
            time.sleep(.15)
            if self.args.native_test and "outcome is unknown" not in self.metrics()["inline"]["status"]:
                raise RuntimeError("Unknown write outcome was overwritten by a no-replacement notice")
            self.f("key", self.native, self.native_title, 13)
            time.sleep(.15)
            after = self.state()
            expected = "pre|" + PAYLOAD + " |post"
            if (before["text"] != expected or after["text"] != expected
                    or after["paste_attempts"] + after["range_replace_attempts"] != 1
                    or after["enter_count"] != 0 or not self.d("exists")):
                raise RuntimeError("Unknown outcome replayed a paste, leaked Enter, or lost the retained content")
        finally:
            self.n("paste-reply-policy", delay_ms=0)
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "unknown-outcome session explicitly cancelled")
        return {"before_repeat": before, "after_repeat": after}

    def test_clipboard_contention(self):
        self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
        self.n("clipboard-contention-policy", enabled=True)
        try:
            result = self.enter(expected="pre|" + PAYLOAD + " |post")
            if not result["clipboard_fault_held"]:
                raise RuntimeError("Competing clipboard reader fault was not exercised")
            if result["range_replace_attempts"] != 1 or result["paste_attempts"] != 0:
                raise RuntimeError("Standard Edit did not perform exactly one native range replacement")
        finally:
            self.n("clipboard-contention-policy", enabled=False)
        self.f("hotkey", self.native, self.native_title, "Ctrl+Z")
        restored = self.wait(lambda: s if (s := self.state())["text"] == "pre|" + QUERY + " |post" else None,
                             "one native Undo restores only the replaced query")
        if restored["enter_count"] != 0:
            raise RuntimeError("Contention or Undo submitted the input")
        return {"replacement": result, "undo": restored}

    def test_delayed_readback(self):
        self.reset(); self.open_inline(); self.type_query(); self.confirm_ready()
        self.n("readback-delay-policy", delay_ms=450)
        try:
            result = self.enter(expected="pre|" + PAYLOAD + " |post")
            if result["readback_delay_count"] != 1 or result["range_replace_attempts"] != 1 or result["paste_attempts"] != 0:
                raise RuntimeError("Delayed-readback fault or single native mutation was not verified")
            return result
        finally:
            self.n("readback-delay-policy", delay_ms=0)

    def test_control(self, control):
        self.reset(control); self.open_inline(); self.type_query()
        return self.enter(control, expected="pre|" + PAYLOAD + " |post")

    def test_password(self):
        for control in ("password", "readonly"):
            before = self.reset(control)["fields"][control]["text"]
            self.f("hotkey", self.native, self.native_title, "Alt+V")
            self.wait(lambda: self.d("exists"), "protected-input fallback")
            self.wait(lambda: "Input filtering unavailable:" in self.d("dump") and "copy manually" in self.d("dump"), "visible manual-copy compatibility notice")
            if "ControlType.Edit | Search clipboard history" in self.d("dump"):
                raise RuntimeError("An unsupported input unexpectedly activated an Echo search field")
            foreground = self.n("state")
            if not foreground["foreground"]:
                raise RuntimeError("Protected input lost foreground: " + repr({
                    key: foreground.get(key) for key in
                    ("foreground_hwnd", "foreground_pid", "foreground_process", "foreground_created_utc")
                }))
            if self.args.native_test and self.metrics()["quick_insert"]["has_target"]:
                raise RuntimeError("Protected input incorrectly admitted a paste target")
            if self.state(control)["text"] != before:
                raise RuntimeError("Protected input was changed")
            self.f("key", self.native, self.native_title, 13)
            time.sleep(.12)
            protected = self.state(control)
            if protected["enter_count"] or protected["text"] != before or not self.d("exists"):
                raise RuntimeError("Protected compatibility surface released Enter protection")
            self.shot("compatibility-" + control)
            if control == "password":
                self.f("key", self.native, self.native_title, 117)
                self.wait(self.manual_history_ready, "F6 retires unavailable-input protection")
                self.wait(lambda: self.n("state").get("foreground_pid") == self.echo.pid,
                          "explicit manual History owns keyboard focus")
                self.f("key", self.echo, TITLE, 27)
            else:
                self.f("key", self.native, self.native_title, 27)
            self.wait(lambda: not self.d("exists"), "Esc retires the protected compatibility surface")
        return "Password/read-only controls retain focus and text; Enter is guarded and Esc dismisses without submission"

    def test_f6(self):
        self.reset(); self.open_inline(); self.type_query()
        self.f("key", self.native, self.native_title, 117)
        self.wait(self.manual_history_ready, "unfiltered manual history")
        if self.state()["text"] != "pre|" + QUERY + " |post":
            raise RuntimeError("F6 deleted the original query")
        return "Typed query preserved; unfiltered history opened without a search field or paste target"

    def test_cancel_matrix(self):
        results = []
        for query in ("", QUERY):
            for cancel in ("escape-repeat", "second-hotkey", "f6"):
                self.reset(); self.open_inline()
                if query:
                    self.type_query(query)
                expected = "pre|" + query + " |post"
                if cancel == "second-hotkey":
                    self.f("hotkey", self.native, self.native_title, "Alt+V")
                elif cancel == "escape-repeat":
                    self.f("key", self.native, self.native_title, 27, 8)
                else:
                    self.f("key", self.native, self.native_title, 117)
                    self.wait(self.manual_history_ready,
                              "explicit F6 manual mode")
                    self.d("close")
                    self.n("focus", control="single")
                self.wait(lambda: not self.d("exists"), "cancellation hides the old surface")
                after = self.state()
                if after["text"] != expected or after["enter_count"]:
                    raise RuntimeError("Cancel removed query text or submitted the host")
                # Re-arm at the current caret without resetting the original
                # document, then cancel again and verify a fresh host Enter.
                self.open_inline()
                self.f("key", self.native, self.native_title, 27)
                self.wait(lambda: not self.d("exists"), "re-armed session cancelled")
                self.f("key", self.native, self.native_title, 13)
                restored = self.wait(lambda: s if (s := self.state())["enter_count"] == 1 else None,
                                     "fresh host Enter after cancel and re-arm")
                if restored["text"] != expected:
                    raise RuntimeError("Cancelled lifecycle changed the original query")
                results.append({"query": query, "cancel": cancel, "protected": after,
                                "normal_after_cancel": restored["enter_count"]})
        return results

    def browser_state(self):
        return json.loads(self.f("read-control", self.browser, self.browser_title, "Browser fixture state"))

    def browser_reset(self, control="search"):
        if self.d("exists"):
            self.d("close")
            self.wait(lambda: not self.d("exists"), "Echo hidden before browser reset")
        label = {"search": "Reset search input", "textarea": "Reset textarea composer", "ai": "Reset rich composer"}[control]
        # Only an owned, visible, unobscured control is clicked. The native-test
        # browser is raised temporarily and normal Z order is restored afterwards.
        if not self.f("geometry", self.browser, self.browser_title)["foreground"]:
            self.f("activate-owned", self.browser, self.browser_title, "Echo Inline Composer Fixture")
        self.f("english-owned", self.browser, self.browser_title)
        self.f("invoke-control", self.browser, self.browser_title, label)
        state = self.wait(lambda: (s if s[control]["focused"] else None) if (s := self.browser_state()) else None,
                          "owned browser input focused", 8)
        self.wait(lambda: self.f("geometry", self.browser, self.browser_title)["foreground"], "browser foreground")
        return state

    def browser_open(self, query=""):
        self.f("hotkey", self.browser, self.browser_title, "Alt+V")
        result = self.wait(lambda: self.inline_ready(query), "browser inline session", 12)
        if not self.f("geometry", self.browser, self.browser_title)["foreground"]:
            raise RuntimeError("Inline popup activated instead of retaining browser input focus")
        return result

    def browser_query(self, query=QUERY):
        self.f("text", self.browser, self.browser_title, query)
        return self.wait(lambda: self.inline_ready(query), "browser query filtered", 10)

    def browser_enter(self, control="search", held=False, expected=None):
        self.confirm_ready()
        before = self.browser_state()
        self.f("held-enter" if held else "key", self.browser, self.browser_title, *([] if held else [13]))
        self.wait(lambda: not self.d("exists"), "browser replacement acknowledged", 10)
        state = self.browser_state()
        if state[control]["submitted"] != 0:
            raise RuntimeError("Enter submitted the browser input")
        if expected is not None and state[control]["text"] != expected:
            raise RuntimeError(f"Browser replacement incorrect: {state[control]['text']!r}")
        if state["mention"] != before["mention"] or state["attachment"] != before["attachment"]:
            raise RuntimeError("Inline replacement destroyed a mention or attachment")
        for other in ("search", "textarea", "ai"):
            if other != control and state[other]["text"] != before[other]["text"]:
                raise RuntimeError("Inline replacement changed another editor")
        return state

    def browser_checks(self):
        if self.echo is None:
            self.start_processes()
        relative = "Google/Chrome/Application/chrome.exe" if self.args.browser == "chrome" else "Microsoft/Edge/Application/msedge.exe"
        roots = [Path(os.environ[name]) for name in ("ProgramFiles", "ProgramFiles(x86)", "ProgramW6432") if os.environ.get(name)]
        # Some remote shells omit ProgramFiles(x86) despite a 32-bit Edge install.
        roots.append(Path(os.environ.get("SystemDrive", "C:") + "/") / "Program Files (x86)")
        browser = next((base / relative for base in roots if (base / relative).is_file()), None)
        if browser is None:
            raise RuntimeError("Requested browser executable was not found in installed program directories")
        profile = self.evidence / "isolated-browser-profile"
        html = (self.root / "tests/native/fixtures/inline-composer.html").as_uri()
        browser_arguments = ["--user-data-dir=" + str(profile), "--no-first-run", "--no-default-browser-check",
            "--disable-background-mode", "--disable-background-networking", "--disable-sync", "--disable-extensions",
            "--window-size=1100,1000", "--window-position=600,150", html if self.args.omnibox else "--app=" + html]
        if self.args.force_browser_accessibility:
            browser_arguments.append("--force-renderer-accessibility")
        self.browser = self.launch(browser, browser_arguments, "browser")
        atomic(self.evidence / "browser-environment.json", {
            "actual_application": False, "environment": "chromium-fixture",
            "forced_accessibility": self.args.force_browser_accessibility,
            "external_enter_interceptor": "controlled fixture counts and suppresses submission",
            "executable": str(browser), "arguments": browser_arguments,
            "sha256": hashlib.sha256(browser.read_bytes()).hexdigest(),
            "application_version": file_version(browser),
        })
        def title():
            command = f"$p=Get-Process -Id {self.browser.pid}; $p.MainWindowTitle | ConvertTo-Json -Compress"
            result = subprocess.run(["pwsh", "-NoProfile", "-Command", command], capture_output=True, stdin=subprocess.DEVNULL,
                                    text=True, encoding="utf-8", timeout=8, creationflags=subprocess.CREATE_NO_WINDOW)
            value = json.loads(result.stdout.lstrip("\ufeff")) if result.returncode == 0 else ""
            return value if value and "Echo Inline Composer Fixture" in value else None
        self.browser_title = self.wait(title, "owned browser fixture window", 15)
        self.record(self.browser, self.browser_title)
        (self.evidence/"browser-initial-tree.txt").write_text(self.f("dump-tree",self.browser,self.browser_title),encoding="utf-8")
        # Activate the validated isolated browser before requesting its page
        # tree. A newly launched background Chromium window may not expose the
        # document provider yet; do not force accessibility flags or use a daily profile.
        self.f("activate-owned",self.browser,self.browser_title,"Echo Inline Composer Fixture")
        self.wait(lambda: self.browser_state(), "local browser accessibility tree ready", 15)
        if self.args.omnibox:
            self.check("browser-omnibox-inline-query-keeps-popup-and-never-navigates", self.test_omnibox)
        self.check("browser-query-and-cancel-preserves-clipboard", self.test_browser_query_only)
        self.check("browser-empty-paragraph-first-key-spaces-and-replacement", self.test_empty_rich)
        self.check("browser-decorated-empty-paragraph-first-key", lambda: self.test_empty_rich(True))
        self.check("browser-leaf-decoration-first-key", lambda: self.test_empty_rich("leaf"))
        self.check("browser-leaf-decoration-preserves-literal-text", self.test_leaf_decoration_literal)
        self.check("browser-actual-placeholder-label-is-preserved", self.test_actual_placeholder_label)
        self.check("browser-real-paragraph-and-selected-newline-ranges", self.test_browser_newline_ranges)
        self.check("browser-nbsp-original-query-range", self.test_browser_nbsp)
        self.check("browser-subtree-rebuild-and-same-name-editor", self.test_browser_editor_identity)
        self.check("browser-search-caret-query-exact-replacement", lambda: self.test_browser_control("search"))
        self.check("browser-textarea-composer-exact-replacement", lambda: self.test_browser_control("textarea"))
        self.check("browser-rich-ai-composer-preserves-mention-and-attachment", lambda: self.test_browser_control("ai"))
        self.check("browser-held-enter-does-not-send", self.test_browser_held)
        self.check("browser-empty-results-never-submit", self.test_browser_empty)
        self.check("browser-fast-query-enter-never-uses-stale-row", self.test_browser_fast)
        self.check("browser-undo-is-a-local-replacement", self.test_browser_undo)
        self.check("browser-click-suggestion-does-not-take-focus", self.test_browser_click)
        if self.args.stress:
            self.check("browser-decorated-empty-stress", lambda: self.repeat_case("decorated-empty", 30, lambda: self.test_empty_rich(True)))
            self.check("browser-leaf-decoration-stress", lambda: self.repeat_case("leaf-decoration", 30, lambda: self.test_empty_rich("leaf")))
            self.check("browser-newline-range-stress", lambda: self.repeat_case("browser-newlines", 20, self.test_browser_newline_ranges))
            self.check("browser-nbsp-range-stress", lambda: self.repeat_case("browser-nbsp", 20, self.test_browser_nbsp))
            self.check("browser-editor-identity-stress", lambda: self.repeat_case("browser-editor-identity", 20, self.test_browser_editor_identity))
            self.check("browser-mouse-lifecycle-stress", lambda: self.repeat_case("browser-mouse", 30, self.test_browser_click))
        self.check("browser-profile-cleanup", self.close_browser)

    def test_browser_query_only(self):
        # No confirmation or copy: this gate is safe with an opaque clipboard
        # owner format because it never asks Echo to replace the clipboard.
        user = ctypes.WinDLL("user32")
        user.GetClipboardSequenceNumber.restype = ctypes.c_uint32
        sequence = user.GetClipboardSequenceNumber()
        observations = []
        for control in ("search", "textarea", "ai"):
            self.browser_reset(control)
            self.browser_open()
            query = ""
            for part in ("ec", " ", "prf", " ", "0013"):
                self.f("text", self.browser, self.browser_title, part)
                query += part
                def observed():
                    value = self.inline_ready()
                    return value if value and value["query"].replace("\u00a0", " ") == query else None
                self.wait(observed, "browser query observed without confirmation")
            value = self.metrics()
            if value["snapshot_model_count"] != 1 or value.get("highlighted_rows", 0) != 1:
                raise RuntimeError("Browser multiword query did not find and highlight the exact result")
            before = self.browser_state()
            self.shot("query-only-" + control)
            self.f("key", self.browser, self.browser_title, 27)
            self.wait(lambda: not self.d("exists"), "browser cancellation hides popup")
            after = self.browser_state()
            if after[control]["text"] != before[control]["text"] or any(after[c]["submitted"] for c in ("search", "textarea", "ai")):
                raise RuntimeError("Query cancellation changed or submitted the host input")
            if after["mention"] != before["mention"] or after["attachment"] != before["attachment"]:
                raise RuntimeError("Query filtering changed host decorations")
            if user.GetClipboardSequenceNumber() != sequence:
                raise RuntimeError("Clipboard changed during a no-copy/no-confirmation scenario")
            observations.append({"control": control, "query": query, "rows": 1, "highlighted": True, "cancel_preserved_input": True})
        return {"observations": observations, "clipboard_sequence_unchanged": True, "insertion": "NOT_RUN"}

    def test_omnibox(self):
        # This is a NORMAL owned Chrome window, not --app (which has no omnibox).
        self.browser_reset()
        self.f("hotkey", self.browser, self.browser_title, "Ctrl+L")
        self.f("key", self.browser, self.browser_title, 8)
        before_dump = self.f("dump", self.browser, self.browser_title)
        (self.evidence / "omnibox-before.uia.txt").write_text(before_dump, encoding="utf-8")
        edits = [line.split(" | ")[1] for line in before_dump.splitlines()
                 if line.startswith(("ControlType.Edit | ", "ControlType.ComboBox | ")) and ("Address" in line or "address" in line or "鍦板潃" in line)]
        if len(edits) != 1:
            raise RuntimeError("Could not identify the owned browser address bar uniquely")
        label=edits[0]
        self.wait(lambda:self.f("read-edit",self.browser,self.browser_title,label)=="", "empty address bar before activation")
        bounds=self.f("card",self.browser,self.browser_title,label)
        atomic(self.evidence/"omnibox-range-before.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.browser_open()
        atomic(self.evidence/"omnibox-range-open.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.f("text",self.browser,self.browser_title,QUERY)
        atomic(self.evidence/"omnibox-range-typed.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.wait(lambda:self.inline_ready(QUERY),"omnibox query filtered",10)
        self.wait(lambda: self.metrics()["inline"]["readiness"][7] == 1,
                  "omnibox query and result synchronized",5)
        card=self.f("card",self.echo,TITLE,"History space")
        if card[1] < bounds[3]-3:
            raise RuntimeError("A top address bar must expand below rather than cover or compress against it")
        if card[1]-bounds[3] > 48:
            raise RuntimeError("Echo is detached from the address bar")
        self.shot("omnibox-filtered")
        self.f("key", self.browser, self.browser_title, 13)
        self.wait(lambda:not self.d("exists"),"omnibox replacement acknowledged",10)
        actual=self.f("read-edit",self.browser,self.browser_title,label)
        if actual != PAYLOAD:
            raise RuntimeError("Address bar replacement mismatch: " + repr(actual))
        state=self.browser_state()
        if any(state[c]["submitted"] for c in ("search","textarea","ai")):
            raise RuntimeError("Completion submitted the local fixture")
        # A successfully queried page state proves Enter did not navigate away.
        return {"text":actual,"original_page_still_loaded":True,"input_bounds":bounds,"card":card}

    def test_browser_newline_ranges(self):
        results = []
        for button, middle, typed in (("Select query between real paragraphs", 1, False),
                                     ("Select query between blank paragraphs", 3, False),
                                     ("Select query between real paragraphs", 1, True),
                                     ("Select query after Unicode paragraph", 1, True)):
            self.browser_reset("ai")
            self.f("invoke-control", self.browser, self.browser_title, button)
            if typed:
                self.f("invoke-control", self.browser, self.browser_title, "Prepare typed paragraph query")
            before = self.browser_state()
            if typed:
                self.browser_open(); self.browser_query()
            else:
                self.browser_open(QUERY)
            expected_text = (before["ai"]["text"].replace("lead|", "lead|" + PAYLOAD, 1) if typed else
                             before["ai"]["text"].replace(QUERY, PAYLOAD, 1))
            after = self.browser_enter("ai", expected=expected_text)
            expected = [dict(p) for p in before["paragraphs"]]
            paragraph_text = "lead|" + PAYLOAD + " |tail" if typed else PAYLOAD
            expected[middle] = {"tag": "P", "text": paragraph_text, "html": paragraph_text}
            if after["paragraphs"] != expected:
                raise RuntimeError("Real paragraph or blank br structure was changed outside the query")
            results.append({"variant": button, "typed": typed, "before": before, "after": after})
        self.browser_reset("textarea")
        self.f("invoke-control", self.browser, self.browser_title, "Select real newlines in textarea")
        self.browser_open("\n" + QUERY + "\n")
        results.append(self.browser_enter("textarea", expected="before|" + PAYLOAD + "|after"))
        # Interior p/br can be represented as the preceding separator. Its
        # uncertain mapping must fail before a confirmation can delete a break.
        self.browser_reset("ai")
        self.f("invoke-control", self.browser, self.browser_title, "Select query between real paragraphs")
        self.f("key", self.browser, self.browser_title, 8)
        before = self.browser_state()
        self.f("hotkey", self.browser, self.browser_title, "Alt+V")
        self.wait(lambda: self.d("exists") and "Empty paragraph boundary is ambiguous" in self.d("dump"),
                  "ambiguous interior paragraph is explicitly unavailable")
        self.f("key", self.browser, self.browser_title, 13)
        after = self.browser_state()
        if after["paragraphs"] != before["paragraphs"] or after["ai"]["submitted"]:
            raise RuntimeError("Ambiguous empty paragraph was changed or submitted")
        self.f("key", self.browser, self.browser_title, 27)
        self.wait(lambda: not self.d("exists"), "ambiguous paragraph cancelled")
        results.append({"ambiguous_empty_paragraph": "refused", "after": after})
        return results

    def test_browser_nbsp(self):
        self.browser_reset("ai"); self.browser_open()
        query = "ec\u00a0prf  0013 "
        self.f("text", self.browser, self.browser_title, query)
        # Chromium may expose additional ordinary spaces as NBSP. Compare the
        # observed raw range separately from the normalized search semantics.
        actual = self.browser_state()["ai"]["text"][len("@Teammate pre|"):-len(" |post")]
        self.wait(lambda: self.inline_ready(actual), "raw Chromium NBSP query")
        return {"typed": query, "observed_query": actual,
                "replacement": self.browser_enter("ai", expected="@Teammate pre|" + PAYLOAD + " |post")}

    def test_browser_editor_identity(self):
        if not self.args.native_test:
            raise NotRun("Late focus-event injection requires the diagnostic candidate")
        self.browser_reset("ai")
        self.f("invoke-control", self.browser, self.browser_title, "Reset editor with paragraph rebuild")
        self.browser_open(); self.browser_query("e")
        self.f("text", self.browser, self.browser_title, QUERY[1:])
        self.wait(lambda: self.inline_ready(QUERY), "rebuilt paragraph retains its editor query")
        before = self.browser_state()
        if before["rebuilds"] < 2 or before["ai"]["text"] != QUERY:
            raise RuntimeError("Internal paragraph identity was not rebuilt during the query")
        self.control_request("pause_inline_window_events", paused=True)
        try:
            self.f("invoke-control", self.browser, self.browser_title, "Focus alternate rich editor")
            moved = self.browser_state()
            if not moved["other"]["focused"] or not self.d("exists"):
                raise RuntimeError("Same-name editor switch did not retain the visible fault lease")
            self.f("key", self.browser, self.browser_title, 13)
            self.wait(lambda: not self.d("exists"), "same-name target rejects the old lease")
            after = self.browser_state()
        finally:
            self.control_request("pause_inline_window_events", paused=False)
        if (after["ai"]["text"] != QUERY or after["other"]["text"] != before["other"]["text"]
                or after["ai"]["submitted"] or after["other"]["submitted"]):
            raise RuntimeError("Cross-editor confirmation changed or submitted a same-name editor")
        return {"same_editor_rebuilds": before["rebuilds"], "after_cross_editor": after}

    def test_browser_control(self, control):
        self.browser_reset(control)
        before = self.browser_open()
        self.browser_query()
        self.shot("browser-" + control + "-filtered")
        expected = ("@Teammate pre|" if control == "ai" else "pre|") + PAYLOAD + " |post"
        after = self.browser_enter(control, expected=expected)
        return {"control": control, "provider": before["inline"]["provider"] if self.args.native_test else "capability-checked", "state": after}

    def test_browser_held(self):
        self.browser_reset("ai"); self.browser_open(); self.browser_query()
        return self.browser_enter("ai", held=True, expected="@Teammate pre|" + PAYLOAD + " |post")

    def test_browser_empty(self):
        self.browser_reset(); self.browser_open(); self.browser_query("no-result-inline-123987")
        self.f("key", self.browser, self.browser_title, 13)
        state = self.browser_state()
        if state["search"]["submitted"] or state["search"]["text"] != "pre|no-result-inline-123987 |post" or not self.d("exists"):
            raise RuntimeError("No-result Enter changed or submitted the search field")
        self.f("key", self.browser, self.browser_title, 27)
        self.wait(lambda: not self.d("exists"), "browser Esc cancelled")
        return state

    def test_browser_fast(self):
        self.browser_reset("ai"); self.browser_open()
        self.f("text-enter", self.browser, self.browser_title, "no-result-fast-123987")
        time.sleep(0.2); state = self.browser_state()
        if state["ai"]["submitted"] or state["ai"]["text"] != "@Teammate pre|no-result-fast-123987 |post":
            raise RuntimeError("Fast typing submitted the AI composer or replaced an old result")
        return state

    def test_browser_undo(self):
        self.browser_reset(); self.browser_open(); self.browser_query(); self.browser_enter(expected="pre|" + PAYLOAD + " |post")
        self.f("hotkey", self.browser, self.browser_title, "Ctrl+Z")
        state = self.wait(lambda: (s if s["search"]["text"] == "pre|" + QUERY + " |post" else None) if (s := self.browser_state()) else None,
                          "one Undo restores query without resetting composer")
        return state

    def test_browser_click(self):
        self.browser_reset(); self.browser_open(); self.browser_query()
        self.confirm_ready()
        pointer = self.f("click-owned", self.echo, TITLE, PAYLOAD)
        atomic(self.evidence / "mouse-click-observation.json", pointer)
        self.wait(lambda: not self.d("exists"), "mouse-selected replacement")
        state = self.browser_state()
        if not self.f("geometry", self.browser, self.browser_title)["foreground"] or state["search"]["text"] != "pre|" + PAYLOAD + " |post" or state["search"]["submitted"]:
            raise RuntimeError("Mouse completion stole focus, submitted, or replaced the wrong text")
        return {"state": state, "pointer": pointer}

    def close_browser(self):
        if self.d("exists"):
            self.d("close"); self.wait(lambda: not self.d("exists"), "Echo hidden for browser cleanup")
        if self.browser and self.browser.poll() is None:
            self.f("close-owned", self.browser, self.browser_title)
            self.browser.wait(timeout=10)
        return "Only the isolated test browser was closed; daily profile was not used"

    def cleanup(self):
        errors = []
        if self.browser and self.browser.poll() is None:
            try:
                self.close_browser()
            except Exception:
                pass
        if self.native and self.native.poll() is None:
            try:
                self.n("quit"); self.native.wait(timeout=8)
            except Exception as error:
                errors.append("native fixture cleanup: " + str(error))
        if self.echo and self.echo.poll() is None:
            try:
                secondary = subprocess.run([str(self.args.executable), "--quit"], env=self.env, capture_output=True, timeout=8,
                                           creationflags=subprocess.CREATE_NO_WINDOW)
                if secondary.returncode != 0:
                    raise RuntimeError("Quit handoff failed")
                self.echo.wait(timeout=8)
            except Exception as error:
                errors.append("Echo cleanup: " + str(error))
        for error in errors:
            self.checks.append({"name": "cleanup", "status": "FAIL", "error": error})
        for log in self.logs:
            log.close()
        return errors

def main() -> int:
    parser = argparse.ArgumentParser()
    for name in ("root", "executable", "tools", "template", "evidence"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--renderer", choices=("software",), default="software")
    parser.add_argument("--native-test", action="store_true")
    parser.add_argument("--record-screen", action="store_true")
    parser.add_argument("--stress", action="store_true", help="Run the explicit minimum-count native lifecycle and failure suites")
    parser.add_argument("--source-snapshot-id", default="", help="Recorded immutable source checkpoint for this executable")
    parser.add_argument("--browser-only", action="store_true")
    parser.add_argument("--application-target", type=Path, help="One preverified shared application draft registration; never a cleanup target")
    parser.add_argument("--application-rounds", type=int, default=30)
    parser.add_argument("--omnibox", action="store_true")
    parser.add_argument("--only", default="")
    parser.add_argument("--browser", choices=("none", "chrome", "edge"), default="none")
    parser.add_argument("--force-browser-accessibility", action="store_true", help="L2 diagnostic only; excluded from ordinary application evidence")
    args = parser.parse_args()
    if args.source_snapshot_id and (len(args.source_snapshot_id) != 64 or
            any(c not in "0123456789abcdef" for c in args.source_snapshot_id)):
        raise RuntimeError("Source checkpoint must be a SHA256 identifier")
    if os.environ.get("ECHO_WINDOWS_ACCEPTANCE") != "1":
        raise RuntimeError("Explicit acceptance authorization required")
    print("START inline acceptance", datetime.now(timezone.utc).isoformat(), flush=True)
    run = Run(args)
    success = False
    try:
        if args.application_target:
            if not 1 <= args.application_rounds <= 100:
                raise RuntimeError("Actual application rounds must be between 1 and 100")
            run.application_checks()
        elif not args.browser_only:
            run.native_checks()
        if not args.application_target and args.browser != "none":
            run.browser_checks()
        success = bool(run.checks) and all(c["status"] == "PASS" for c in run.checks)
    except Exception as error:
        print("FAIL", str(error), flush=True)
        run.checks.append({"name": "run-failure", "status": "FAIL", "error": str(error)})
    finally:
        if run.cleanup():
            success = False
        atomic(run.evidence / "summary.json", {
            "schema": "echo.inline.acceptance.v1", "status": "PASS" if success else "FAIL",
            "started": run.started, "finished": datetime.now(timezone.utc).isoformat(), "checks": run.checks,
            "limitations": [
                {"name": "physical-ime", "status": "NOT_RUN", "reason": "Synthetic input is not physical Chinese IME evidence"},
                {"name": "physical-mixed-dpi", "status": "NOT_RUN", "reason": "Only the actual display configuration is tested"},
                {"name": "all-composers", "status": "NOT_RUN", "reason": "Capability-based behavior is not a universal application certification"},
            ],
        })
    return 0 if success else 1

if __name__ == "__main__":
    with ForegroundLease():
        raise SystemExit(main())
