"""Run the owned WPF/TSF caret fixture through the real Echo observer DLL.

RealTSF cases, fixture-only MockProviderLifecycle callbacks, and the legacy
DiagnosticTransport error-code adapter are recorded separately. The diagnostic
adapter still runs through the real injected DLL, scheduler window, mailbox,
target HWND and diagnostics; it does not replace lifecycle or RealTSF evidence.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def wait_for(predicate, label, timeout=10, interval=0.025):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            last = predicate()
            if last:
                return last
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(interval)
    raise RuntimeError(f"timeout waiting for {label}: {last}")


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def loaded_modules(pid):
    """Enumerate modules actually loaded in one target process."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    snapshot = kernel32.CreateToolhelp32Snapshot
    snapshot.argtypes = [ctypes.c_uint32, ctypes.c_uint32]
    snapshot.restype = ctypes.c_void_p
    first = kernel32.Module32FirstW
    next_module = kernel32.Module32NextW
    close = kernel32.CloseHandle
    first.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    first.restype = ctypes.c_int
    next_module.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    next_module.restype = ctypes.c_int
    close.argtypes = [ctypes.c_void_p]
    close.restype = ctypes.c_int

    class MODULEENTRY32W(ctypes.Structure):
        _fields_ = [
            ("dwSize", ctypes.c_uint32),
            ("th32ModuleID", ctypes.c_uint32),
            ("th32ProcessID", ctypes.c_uint32),
            ("GlblcntUsage", ctypes.c_uint32),
            ("ProccntUsage", ctypes.c_uint32),
            ("modBaseAddr", ctypes.c_void_p),
            ("modBaseSize", ctypes.c_uint32),
            ("hModule", ctypes.c_void_p),
            ("szModule", ctypes.c_wchar * 256),
            ("szExePath", ctypes.c_wchar * 260),
        ]

    INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
    handle = snapshot(0x00000018, pid)  # TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32
    if not handle or handle == INVALID_HANDLE_VALUE:
        raise ctypes.WinError(ctypes.get_last_error())
    result = []
    try:
        entry = MODULEENTRY32W()
        entry.dwSize = ctypes.sizeof(entry)
        if not first(handle, ctypes.byref(entry)):
            return result
        while True:
            result.append({"name": entry.szModule, "path": entry.szExePath})
            if not next_module(handle, ctypes.byref(entry)):
                break
        return result
    finally:
        close(handle)


def compare_rect(actual, expected, tolerance=2):
    return (
        isinstance(actual, list)
        and isinstance(expected, list)
        and len(actual) == len(expected) == 4
        and all(abs(int(a) - int(e)) <= tolerance for a, e in zip(actual, expected))
    )


def expected_badge(sample):
    geometry = sample["geometry"]
    target = geometry["rect"]
    work = geometry["work_area"]
    dpi = int(geometry["dpi"] or 96)
    scale = lambda dip: max(1, round(dip * dpi / 96.0))
    width, height, gap = scale(48), scale(36), scale(8)
    work_x, work_y, work_width, work_height = map(int, work)
    target_x, target_y, target_width, target_height = map(int, target)
    right = work_x + work_width
    bottom = work_y + work_height
    if sample.get("anchor") == "Control":
        x = target_x + target_width - width
    else:
        x = target_x + target_width + gap
        if x + width > right:
            x = target_x - gap - width
    above = target_y - gap - height
    y = above if above >= work_y else target_y + target_height + gap
    return [
        max(work_x, min(x, right - width)),
        max(work_y, min(y, bottom - height)),
        width,
        height,
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--observer-dll", type=Path)
    parser.add_argument("--provider", choices=("shadow", "primary", "legacy"), default="primary")
    args = parser.parse_args()
    if os.environ.get("ECHO_WINDOWS_ACCEPTANCE") != "1":
        raise RuntimeError("Explicit native acceptance authorization required")
    if args.evidence.exists():
        raise RuntimeError("--evidence must be a new path")
    root = args.evidence.resolve()
    root.mkdir(parents=True, exist_ok=False)
    data = root / "data"
    data.mkdir()
    (data / "synthetic-fixture.json").write_text(
        json.dumps({"synthetic": True, "capture_enabled": False}), encoding="utf-8"
    )
    executable = args.executable.resolve()
    fixture = args.fixture.resolve()
    if not executable.exists() or not fixture.exists():
        raise RuntimeError("executable and fixture must exist")
    shutil.copy2(executable, root / "echo-acceptance.exe")

    observer_info = None
    if args.observer_dll:
        observer = args.observer_dll.resolve()
        if not observer.exists():
            raise RuntimeError(f"observer DLL does not exist: {observer}")
        observer_info = {"path": str(observer), "sha256": sha256(observer), "exists": True}

    if fixture.suffix.lower() == ".cs":
        compiler = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
        fixture_exe = root / "CaretTsfFixture.exe"
        references = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Reference Assemblies/Microsoft/Framework/.NETFramework/v4.8"
        subprocess.run(
            [str(compiler), "/nologo", "/target:winexe", "/out:" + str(fixture_exe),
             "/reference:" + str(references / "PresentationCore.dll"),
             "/reference:" + str(references / "PresentationFramework.dll"),
             "/reference:" + str(references / "WindowsBase.dll"),
             "/reference:" + str(references / "System.Xaml.dll"),
             "/reference:System.dll", str(fixture)],
            check=True, creationflags=subprocess.CREATE_NO_WINDOW,
        )
        fixture = fixture_exe

    env = dict(
        os.environ,
        ECHO_DATA_DIR=str(data),
        ECHO_NATIVE_TEST_ROOT=str(root),
        ECHO_RENDERER="software",
        ECHO_CARET_PROVIDER=args.provider,
        ECHO_CARET_FAULT_FILE=str(root / "caret-fault.txt"),
    )
    children = []
    checks = []
    sequence = 0
    ready = None
    echo_log = None
    fixture_log = None
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.SetForegroundWindow.argtypes = [ctypes.c_void_p]
    user32.SetForegroundWindow.restype = ctypes.c_int
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    user32.GetAncestor.argtypes = [ctypes.c_void_p, ctypes.c_uint]
    user32.GetAncestor.restype = ctypes.c_void_p
    user32.GetWindowThreadProcessId.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
    user32.GetWindowThreadProcessId.restype = ctypes.c_uint32
    user32.keybd_event.argtypes = [ctypes.c_ubyte, ctypes.c_ubyte, ctypes.c_ulong, ctypes.c_void_p]

    def assert_owned_fixture_foreground(label):
        if ready is None:
            raise RuntimeError(f"{label}: fixture is not ready")
        foreground = int(user32.GetForegroundWindow())
        if foreground != int(ready["hwnd"]):
            raise RuntimeError(
                f"{label}: owned fixture lost foreground; refusing cross-process observer "
                f"(foreground={foreground}, fixture={ready['hwnd']})"
            )

    def fixture_command(command):
        if ready is None:
            raise RuntimeError("fixture is not ready")
        assert_owned_fixture_foreground(f"fixture command {command}")
        nonce = uuid.uuid4().hex
        command_path = root / ready.get("command", "fixture.command.json")
        response_path = root / ready.get("response", "fixture.response.json")
        try:
            response_path.unlink()
        except FileNotFoundError:
            pass
        temp = command_path.with_suffix(".tmp")
        temp.write_text(nonce + "|" + command, encoding="utf-8")
        os.replace(temp, command_path)
        response = wait_for(lambda: read_json(response_path), "fixture response", timeout=5)
        if response.get("nonce") != nonce or response.get("command") != command:
            raise RuntimeError(f"fixture response identity mismatch: {response}")
        response_path.unlink(missing_ok=True)
        return response

    def echo_call(command):
        nonlocal sequence
        assert_owned_fixture_foreground(f"Echo command {command.get('verb', 'unknown')}")
        sequence += 1
        request = root / "native-control" / "request.json"
        response = root / "native-control" / "response.json"
        payload = dict(command, id=str(sequence), pid=echo.pid)
        try:
            response.unlink()
        except FileNotFoundError:
            pass
        except PermissionError:
            for _ in range(40):
                try:
                    response.unlink()
                    break
                except FileNotFoundError:
                    break
                except PermissionError:
                    time.sleep(0.005)
        temp = request.with_suffix(".tmp")
        temp.write_text(json.dumps(payload), encoding="utf-8")
        for _ in range(80):
            try:
                os.replace(temp, request)
                break
            except PermissionError:
                time.sleep(0.005)
        else:
            raise RuntimeError("native control request remained locked")

        def read():
            value = read_json(response)
            return value if value.get("id") == str(sequence) else None

        value = wait_for(read, f"Echo response {sequence}", timeout=8)
        if value.get("status") != "PASS":
            raise RuntimeError(value)
        return value["value"]

    def oracle(after=None):
        value = read_json(root / "fixture.oracle.json")
        return value if after is None or int(value.get("sequence", 0)) > after else None

    def check(name, status, **details):
        entry = {"name": name, "status": status, "artifact": "checks.json"}
        entry.update(details)
        checks.append(entry)
        (root / "checks.json").write_text(json.dumps(checks, indent=2), encoding="utf-8")
        print(status, name, flush=True)
        return entry

    def trace_counters(value, allow_hidden=False):
        trace = value.get("tsf")
        if not isinstance(trace, dict):
            if allow_hidden and not value.get("visible", True):
                # Deep hide releases the display-side diagnostic snapshot.
                # The last valid TSF snapshot remains the authoritative
                # counter boundary; a missing object here is not a missing
                # field in an existing diagnostic record.
                return None
            raise AssertionError(f"TSF counters missing: {value}")
        names = (
            "api_requests",
            "request_edit_calls",
            "accepted_sessions",
            "callback_entered",
            "callback_completed",
            "final_released",
            "created_callbacks",
            "released_callbacks",
            "cancelled",
            "timed_out",
            "ready_results",
            "mock_sessions",
            "mock_callback_entered",
            "mock_callback_completed",
            "mock_final_released",
        )
        counters = {}
        for name in names:
            if name not in trace or trace[name] is None:
                raise AssertionError(f"required TSF counter missing: {name} trace={trace}")
            counters[name] = int(trace[name])
        for name in ("callback_high_water", "pending_high_water", "pending_callbacks", "outstanding_callbacks"):
            if name not in trace or trace[name] is None:
                raise AssertionError(f"required TSF high-water field missing: {name} trace={trace}")
            counters[name] = int(trace[name])
        return counters

    def counter_delta(before, after):
        return {name: int(after[name]) - int(before[name]) for name in before}

    def wait_indicator(predicate, label, timeout=8):
        latest = None

        def probe():
            nonlocal latest
            latest = echo_call({"verb": "input_indicator"})
            return latest if latest and predicate(latest) else None

        try:
            return wait_for(probe, label, timeout=timeout, interval=0.08)
        except RuntimeError as error:
            raise RuntimeError(f"{error}; last_indicator={latest}") from error

    def assert_oracle(value):
        required = (
            "pid", "hwnd", "input_hwnd", "visible", "dpi", "window_rect",
            "input_rect", "work_area", "caret", "sequence",
        )
        if any(key not in value for key in required):
            raise AssertionError(f"oracle missing fields: {value}")
        if value["pid"] != fixture_process.pid or int(value["hwnd"]) != int(ready["hwnd"]):
            raise AssertionError(f"oracle ownership mismatch: {value}")
        input_hwnd = int(value["input_hwnd"])
        root_hwnd = int(value["hwnd"])
        if not value["visible"] or not input_hwnd:
            raise AssertionError(f"oracle HWND is not a visible owned target: {value}")
        if input_hwnd == root_hwnd:
            raise AssertionError(f"oracle input HWND must be an independent child: {value}")
        if int(user32.GetAncestor(ctypes.c_void_p(input_hwnd), 2)) != root_hwnd:
            raise AssertionError(f"oracle child HWND is outside root: {value}")
        owner = ctypes.c_uint32()
        if user32.GetWindowThreadProcessId(ctypes.c_void_p(input_hwnd), ctypes.byref(owner)) == 0 or owner.value != fixture_process.pid:
            raise AssertionError(f"oracle child HWND owner mismatch: {value}")
        if len(value["caret"]) != 4 or value["caret"][2] <= value["caret"][0] or value["caret"][3] <= value["caret"][1]:
            raise AssertionError(f"oracle caret is invalid: {value}")
        if len(value["work_area"]) != 4 or value["work_area"][2] <= 0 or value["work_area"][3] <= 0:
            raise AssertionError(f"oracle work area is invalid: {value}")
        if len(value["input_rect"]) != 4 or value["input_rect"][2] <= 0 or value["input_rect"][3] <= 0:
            raise AssertionError(f"oracle child rect is invalid: {value}")

    try:
        fixture_log = (root / "fixture.log").open("w", encoding="utf-8")
        fixture_process = subprocess.Popen([str(fixture), str(root)], env=env,
                                           stdout=fixture_log, stderr=fixture_log,
                                           creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(fixture_process)
        wait_for(lambda: (root / "fixture.ready.json").exists(), "owned WPF fixture", timeout=12)
        ready = read_json(root / "fixture.ready.json")
        if int(ready["pid"]) != fixture_process.pid:
            raise RuntimeError(f"fixture PID changed: {ready}")
        # Bind the observer to this exact fixture before starting Echo. The
        # native monitor fails closed if foreground moves to another process.
        env["ECHO_NATIVE_TEST_TARGET_PID"] = str(fixture_process.pid)
        env["ECHO_NATIVE_TEST_TARGET_HWND"] = str(ready["hwnd"])
        if int(user32.GetForegroundWindow()) != int(ready["hwnd"]):
            # Windows foreground-lock policy permits the owned fixture to be
            # activated after a bounded Alt tap. This does not send text.
            user32.keybd_event(0x12, 0, 0, None)
            user32.keybd_event(0x12, 0, 2, None)
            user32.SetForegroundWindow(ctypes.c_void_p(ready["hwnd"]))
        wait_for(lambda: int(user32.GetForegroundWindow()) == int(ready["hwnd"]), "owned fixture foreground", timeout=5)

        # Start Echo only after the owned target is foreground. This prevents
        # the first primary-provider poll from installing a hook in whatever
        # desktop app happened to be active before the fixture launched.
        echo_log = (root / "echo.log").open("w", encoding="utf-8")
        echo = subprocess.Popen([str(root / "echo-acceptance.exe"), "--background"], env=env,
                                stdout=echo_log, stderr=echo_log, creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(echo)
        wait_for(lambda: (root / "native-control").is_dir(), "Echo native bridge", timeout=12)

        # Never steal focus after Echo starts. A lost target aborts the run so
        # the observer cannot sample an unrelated browser or editor.
        assert_owned_fixture_foreground("after Echo startup")

        normal = fixture_command("CreateScenario|normal")
        if normal.get("status") != "PASS":
            raise RuntimeError(normal)
        fixture_command("FocusEditor")
        current_oracle = wait_for(lambda: oracle(), "initial fixture oracle", timeout=5)
        assert_oracle(current_oracle)
        (root / "oracle-normal.json").write_text(json.dumps(current_oracle, indent=2), encoding="utf-8")
        check("N01-owned-fixture-oracle", "PASS", oracle=current_oracle, fixture_hwnd=ready["hwnd"])

        def validate_ready(name, oracle_value, value, previous=None):
            sample = value.get("sample") or {}
            geometry = sample.get("geometry") or {}
            tsf = value.get("tsf") or {}
            if not value.get("visible") or not value.get("hwnd"):
                raise AssertionError(f"badge not visible/owned: {value}")
            if sample.get("window") != oracle_value["hwnd"]:
                raise AssertionError(f"sample target mismatch: {value}")
            if geometry.get("source") != "TsfCaret":
                raise AssertionError(f"selected source is not TsfCaret: {value}")
            if not compare_rect(geometry.get("work_area"), oracle_value["work_area"], tolerance=0):
                raise AssertionError(f"host work-area does not match independent oracle: {geometry} vs {oracle_value}")
            if int(geometry.get("dpi") or 0) != int(oracle_value["dpi"]):
                raise AssertionError(f"host monitor DPI does not match independent oracle: {geometry} vs {oracle_value}")
            if tsf.get("state") != "Ready" or not tsf.get("raw_rect") or not tsf.get("normalized_rect"):
                raise AssertionError(f"TSF diagnostics not ready: {value}")
            if not compare_rect(tsf["normalized_rect"], oracle_value["caret"], tolerance=4):
                raise AssertionError(f"TSF/oracle mismatch: {tsf} vs {oracle_value}")
            expected = expected_badge(sample)
            if not compare_rect(value.get("rect"), expected, tolerance=2):
                raise AssertionError(f"badge placement mismatch: {value.get('rect')} vs {expected}")
            if not compare_rect(value.get("actual_hwnd_rect"), expected, tolerance=2):
                raise AssertionError(f"native HWND placement mismatch: {value.get('actual_hwnd_rect')} vs {expected}")
            if previous is not None and tsf.get("context_epoch") == previous:
                raise AssertionError(f"context epoch did not change: {value}")
            check(name, "PASS", kind="RealTSF", oracle=oracle_value, indicator=value, expected_badge=expected)
            return value

        initial = wait_indicator(lambda value: (value.get("sample") or {}).get("geometry", {}).get("source") == "TsfCaret", "initial TSF caret")
        validate_ready("G2-real-tsf-window-and-coordinate-chain", current_oracle, initial)

        def capture_observer_identity():
            """Capture the injected observer while the real TSF observer is live."""
            if observer_info is None:
                return None
            try:
                modules = loaded_modules(fixture_process.pid)
                matching = [
                    module
                    for module in modules
                    if module.get("path")
                    and Path(module["path"]).exists()
                    and sha256(Path(module["path"])) == observer_info["sha256"]
                ]
                observer_info["target_pid"] = fixture_process.pid
                observer_info["modules_enumerated"] = True
                observer_info["loaded_modules"] = matching
                observer_info["loaded"] = bool(matching)
                return observer_info if matching else None
            except OSError as error:
                observer_info["target_pid"] = fixture_process.pid
                observer_info["modules_enumerated"] = False
                observer_info["loaded"] = "UNAVAILABLE"
                observer_info["load_error"] = str(error)
                return None

        # SetWindowsHookExW loads the observer into the target thread. Capture
        # the module identity during the live TSF window; a later lifecycle
        # close can unload it before the end-of-run report is written.
        if observer_info is not None:
            try:
                wait_for(capture_observer_identity, "observer DLL module identity", timeout=5)
            except RuntimeError:
                # Preserve the last enumeration in the final diagnostic even
                # when the target does not expose the module list to this run.
                capture_observer_identity()

        scenarios = [
            ("N02-empty-context", "CreateScenario|empty", True),
            ("N05-readonly", "CreateScenario|readonly", False),
            ("N11-context-replaced", "CreateScenario|context-replaced", True),
            ("N12-root-child-two", "CreateScenario|child-two", True),
            ("N13-root-child-one", "CreateScenario|child-one", True),
            ("N20-current-dpi-smoke", "CreateScenario|mixed-dpi", True),
            ("N23-mode-only", "SetSyntheticModeForFixture|normal", True),
            ("N24-geometry-only", "MoveCaret|0", True),
        ]
        last_epoch = (initial.get("tsf") or {}).get("context_epoch")
        for name, command, require_ready in scenarios:
            before = int(current_oracle["sequence"])
            previous_input_hwnd = int(current_oracle["input_hwnd"])
            result = fixture_command(command)
            if result.get("status") == "UNSUPPORTED":
                check(name, "UNSUPPORTED", reason=result.get("reason"))
                continue
            if result.get("status") != "PASS":
                check(name, "FAIL", reason=result)
                continue
            fixture_command("FocusEditor")
            current_oracle = wait_for(lambda: oracle(before), name + " oracle", timeout=5)
            assert_oracle(current_oracle)
            (root / (name + ".oracle.json")).write_text(json.dumps(current_oracle, indent=2), encoding="utf-8")
            if require_ready:
                try:
                    def current_ready(value):
                        sample = value.get("sample") or {}
                        geometry = sample.get("geometry") or {}
                        tsf = value.get("tsf") or {}
                        return (
                            geometry.get("source") == "TsfCaret"
                            and tsf.get("state") == "Ready"
                            and compare_rect(tsf.get("normalized_rect"), current_oracle["caret"], tolerance=4)
                        )
                    value = wait_indicator(current_ready, name + " TSF")
                    epoch = (value.get("tsf") or {}).get("context_epoch")
                    validate_ready(
                        name,
                        current_oracle,
                        value,
                        last_epoch if name in ("N11-context-replaced", "N12-root-child-two", "N13-root-child-one") else None,
                    )
                    if name in ("N12-root-child-two", "N13-root-child-one") and int(current_oracle["input_hwnd"]) == previous_input_hwnd:
                        raise AssertionError(f"child switch did not change input HWND: {current_oracle}")
                    last_epoch = epoch or last_epoch
                except (AssertionError, RuntimeError) as error:
                    check(name, "FAIL", error=str(error), oracle=current_oracle)
            else:
                try:
                    value = wait_indicator(
                        lambda value: (value.get("tsf") or {}).get("state") == "Unavailable",
                        name + " TSF refusal",
                    )
                    reason = (value.get("tsf") or {}).get("reason_code")
                    if name == "N05-readonly" and reason != 12:
                        raise AssertionError(f"expected readonly reason 12, got {reason}")
                    check(name, "PASS", oracle=current_oracle, indicator=value)
                except (AssertionError, RuntimeError) as error:
                    check(name, "FAIL", error=str(error), oracle=current_oracle)

        # N18 is the real lifecycle case: repeatedly disable and re-enable the
        # indicator through the native test bridge, then prove that the same
        # owned target can recover a fresh TSF sample. This does not substitute
        # for the unavailable COM fault-adapter cases below.
        before_toggle = echo_call({"verb": "input_indicator"})
        toggle_count = 100
        toggle_results = []
        for _ in range(toggle_count):
            disabled = echo_call({"verb": "input_indicator_setting", "paused": True})
            enabled = echo_call({"verb": "input_indicator_setting", "paused": False})
            if disabled.get("draft") is not False or enabled.get("draft") is not True:
                raise RuntimeError("input indicator toggle did not apply")
            toggle_results.append((disabled.get("draft"), enabled.get("draft")))
        after_toggle = wait_indicator(
            lambda value: (value.get("sample") or {}).get("geometry", {}).get("source") == "TsfCaret",
            "N18-repeated-enable-disable TSF",
        )
        check(
            "N18-repeated-enable-disable",
            "PASS",
            toggles=toggle_count,
            before_counts=before_toggle.get("counts"),
            after_counts=after_toggle.get("counts"),
            indicator=after_toggle,
        )

        before = int(current_oracle["sequence"])
        result = fixture_command("CreateScenario|multiline")
        if result.get("status") == "PASS":
            fixture_command("FocusEditor")
            current_oracle = wait_for(lambda: oracle(before), "multiline fixture oracle", timeout=5)
            assert_oracle(current_oracle)
            try:
                value = wait_indicator(
                    lambda value: (
                        (value.get("sample") or {}).get("geometry", {}).get("source") == "TsfCaret"
                        and compare_rect(
                            (value.get("tsf") or {}).get("normalized_rect"),
                            current_oracle["caret"],
                            tolerance=4,
                        )
                    ),
                    "multiline TSF",
                )
                check("fixture-multiline-real-tsf", "PASS", oracle=current_oracle, indicator=value)
            except (AssertionError, RuntimeError) as error:
                check("fixture-multiline-real-tsf", "FAIL", error=str(error), oracle=current_oracle)
        else:
            check("fixture-multiline-real-tsf", "UNSUPPORTED", reason=result.get("reason", result))

        # G3 stress terminates on target-owned accepted sessions, not RPC read
        # count or attempted API requests.  The monitor is then *actually*
        # disabled and we wait for a fresh post-stop diagnostic snapshot.  A
        # pre-stop snapshot is never reused as evidence that callbacks drained.
        stress_count = 1000
        stress_started = time.monotonic()
        stress_before = echo_call({"verb": "input_indicator"})
        stress_before_counters = trace_counters(stress_before)
        stress_before_sample = stress_before.get("sample") or {}
        if not isinstance(stress_before_sample, dict):
            raise AssertionError(f"G3 target identity missing from starting sample: {stress_before}")
        stress_target = {
            "pid": int(stress_before_sample.get("pid") or 0),
            "window": int(stress_before_sample.get("window") or 0),
            "focused_window": int(stress_before_sample.get("focused_window") or 0),
            "fixture_pid": int(fixture_process.pid),
            "fixture_root_hwnd": int(ready["hwnd"]),
        }
        if stress_target["pid"] != fixture_process.pid or stress_target["window"] != int(ready["hwnd"]):
            raise AssertionError(f"G3 target identity mismatch at start: {stress_target}")

        def check_stress_sample(value, label):
            sample = value.get("sample") or {}
            if not sample:
                return
            if int(sample.get("pid") or 0) != stress_target["pid"]:
                raise AssertionError(f"{label} changed target PID: {sample}")
            if int(sample.get("window") or 0) != stress_target["window"]:
                raise AssertionError(f"{label} changed target root HWND: {sample}")

        def observe_stress(value, label, allow_hidden=True):
            counters = trace_counters(value, allow_hidden=allow_hidden)
            if counters is None:
                return None
            check_stress_sample(value, label)
            if counters["pending_callbacks"] > 1:
                raise AssertionError(f"{label} pending callback state exceeded 1: {counters}")
            if counters["outstanding_callbacks"] > 8:
                raise AssertionError(f"{label} outstanding callback cap exceeded: {counters}")
            if counters["callback_high_water"] > 8:
                raise AssertionError(f"{label} callback high-water cap exceeded: {counters}")
            if counters["pending_high_water"] > 1:
                raise AssertionError(f"{label} pending high-water cap exceeded: {counters}")
            return counters

        stress_latest = stress_before
        stress_terminal = None
        stress_error = None
        reads = 0
        drain_reads = 0
        max_pending = stress_before_counters["pending_callbacks"]
        max_outstanding = stress_before_counters["outstanding_callbacks"]
        max_high_water = stress_before_counters["callback_high_water"]
        max_pending_high_water = stress_before_counters["pending_high_water"]
        stop_requested_at = None
        drain_finished_at = None

        def set_indicator_enabled(enabled):
            """Persist an isolated monitor toggle and wait for UI state to agree."""
            paused = not enabled
            result = echo_call(
                {"verb": "input_indicator_setting", "paused": paused, "file": "save"}
            )
            expected = bool(enabled)
            if bool(result.get("draft")) != expected:
                raise AssertionError(f"indicator toggle did not apply: {result}")

            def committed():
                state = echo_call(
                    {"verb": "input_indicator_setting", "paused": paused, "file": "cancel"}
                )
                return state if bool(state.get("saved")) == expected and bool(state.get("draft")) == expected else None

            return wait_for(committed, "input indicator monitor setting", timeout=8, interval=0.08)

        try:
            while True:
                stress_latest = echo_call({"verb": "input_indicator"})
                reads += 1
                counters = observe_stress(stress_latest, "G3 stress", allow_hidden=True)
                if counters is None:
                    # Hidden diagnostics contain no current TSF snapshot. Keep
                    # polling, but never substitute a pre-stop counter value.
                    if time.monotonic() - stress_started > 120:
                        raise AssertionError("accepted-session loop lost its TSF diagnostic")
                    continue
                max_pending = max(max_pending, counters["pending_callbacks"])
                max_outstanding = max(max_outstanding, counters["outstanding_callbacks"])
                max_high_water = max(max_high_water, counters["callback_high_water"])
                max_pending_high_water = max(max_pending_high_water, counters["pending_high_water"])
                accepted_delta = counters["accepted_sessions"] - stress_before_counters["accepted_sessions"]
                if accepted_delta < 0:
                    raise AssertionError(f"accepted-session counter regressed: {counters}")
                if accepted_delta >= stress_count:
                    break
                if time.monotonic() - stress_started > 120:
                    raise AssertionError(
                        f"accepted-session timeout delta={accepted_delta} reads={reads}"
                    )

            # `input_indicator_setting` normally edits a draft.  `file=save`
            # commits the isolated synthetic setting so this stop really tears
            # down the monitor and cannot race another acquisition.  Wait for
            # the commit instead of assuming the command response is enough.
            stop_requested_at = time.monotonic()
            set_indicator_enabled(False)

            # A final Release can arrive after the scheduler close.  Read only
            # snapshots obtained after stop_requested_at and require the
            # target-owned counters to settle; a cached stress snapshot is a
            # hard failure rather than evidence of a drain.
            drain_deadline = time.monotonic() + 10
            while time.monotonic() < drain_deadline:
                candidate = echo_call({"verb": "input_indicator"})
                drain_reads += 1
                counters = observe_stress(candidate, "G3 post-stop drain", allow_hidden=True)
                if counters is None:
                    time.sleep(0.08)
                    continue
                stress_terminal = candidate
                max_pending = max(max_pending, counters["pending_callbacks"])
                max_outstanding = max(max_outstanding, counters["outstanding_callbacks"])
                max_high_water = max(max_high_water, counters["callback_high_water"])
                max_pending_high_water = max(max_pending_high_water, counters["pending_high_water"])
                created_delta = counters["created_callbacks"] - stress_before_counters["created_callbacks"]
                released_delta = counters["released_callbacks"] - stress_before_counters["released_callbacks"]
                final_delta = counters["final_released"] - stress_before_counters["final_released"]
                if (
                    counters["pending_callbacks"] == 0
                    and counters["outstanding_callbacks"] == 0
                    and created_delta == released_delta == final_delta
                ):
                    drain_finished_at = time.monotonic()
                    break
                time.sleep(0.08)
            if stress_terminal is None:
                raise AssertionError("G3 drain produced no post-stop TSF diagnostic")
            terminal_counters = trace_counters(stress_terminal)
            if drain_finished_at is None:
                raise AssertionError(
                    f"G3 drain timeout after stop: {terminal_counters} reads={drain_reads}"
                )
            accepted_delta = terminal_counters["accepted_sessions"] - stress_before_counters["accepted_sessions"]
            created_delta = terminal_counters["created_callbacks"] - stress_before_counters["created_callbacks"]
            released_delta = terminal_counters["released_callbacks"] - stress_before_counters["released_callbacks"]
            final_delta = terminal_counters["final_released"] - stress_before_counters["final_released"]
            ready_delta = terminal_counters["ready_results"] - stress_before_counters["ready_results"]
            if accepted_delta < stress_count:
                raise AssertionError(f"accepted-session drain lost: {terminal_counters}")
            if created_delta != released_delta or created_delta != final_delta:
                raise AssertionError(
                    f"callback create/final-release mismatch created={created_delta} released={released_delta} final={final_delta}; counters={terminal_counters}"
                )
            if terminal_counters["outstanding_callbacks"] != 0:
                raise AssertionError(f"G3 outstanding callbacks remained after drain: {terminal_counters}")
            if terminal_counters["pending_callbacks"] != 0:
                raise AssertionError(f"G3 pending callback remained after drain: {terminal_counters}")
            if terminal_counters["callback_entered"] - stress_before_counters["callback_entered"] != terminal_counters["callback_completed"] - stress_before_counters["callback_completed"]:
                raise AssertionError(f"callback entry/completion mismatch: {terminal_counters}")
            if ready_delta < stress_count:
                raise AssertionError(
                    f"1000 accepted sessions did not yield 1000 ready results: accepted={accepted_delta} ready={ready_delta}"
                )
        except (AssertionError, RuntimeError, ValueError) as error:
            stress_error = str(error)
        finally:
            try:
                # Restore the isolated setting for subsequent mock lifecycle
                # cases. This is not evidence for G3 and is never used as its
                # terminal counter snapshot.
                set_indicator_enabled(True)
            except (RuntimeError, OSError):
                pass
        stress_after = stress_terminal
        stress_after_counters = trace_counters(stress_after) if stress_after is not None else None
        stress_delta = counter_delta(stress_before_counters, stress_after_counters) if stress_after_counters else None
        stress_summary = {
            "requested_accepted_sessions": stress_count,
            "attempted_requests": stress_delta["api_requests"] if stress_delta else None,
            "actual_request_edit_session_calls": stress_delta["request_edit_calls"] if stress_delta else None,
            "accepted_sessions": stress_delta["accepted_sessions"] if stress_delta else None,
            "created_callbacks": stress_delta["created_callbacks"] if stress_delta else None,
            "released_callbacks": stress_delta["released_callbacks"] if stress_delta else None,
            "callback_entered": stress_delta["callback_entered"] if stress_delta else None,
            "callback_completed": stress_delta["callback_completed"] if stress_delta else None,
            "final_released": stress_delta["final_released"] if stress_delta else None,
            "ready_results": stress_delta["ready_results"] if stress_delta else None,
            "cancelled": stress_delta["cancelled"] if stress_delta else None,
            "timed_out": stress_delta["timed_out"] if stress_delta else None,
            "outstanding_callbacks_at_drain": stress_after_counters["outstanding_callbacks"] if stress_after_counters else None,
            "pending_callbacks_at_drain": stress_after_counters["pending_callbacks"] if stress_after_counters else None,
            "observation_reads": reads,
            "drain_reads": drain_reads,
            "post_stop_snapshot": stress_terminal is not None,
            "drain_completed": drain_finished_at is not None,
            "elapsed_ms": round((time.monotonic() - stress_started) * 1000),
            "stop_wait_ms": round((drain_finished_at - stop_requested_at) * 1000) if drain_finished_at and stop_requested_at else None,
            "before_counts": stress_before.get("counts"),
            "after_counts": stress_after.get("counts") if stress_after else None,
            "before_tsf_counters": stress_before_counters,
            "after_tsf_counters": stress_after_counters,
            "counter_delta": stress_delta,
            "target_identity": stress_target,
            "max_pending_callbacks": max_pending,
            "max_outstanding_callbacks": max_outstanding,
            "max_callback_high_water": max_high_water,
            "max_pending_high_water": max_pending_high_water,
            "error": stress_error,
        }
        (root / "g3-stress.json").write_text(json.dumps(stress_summary, indent=2), encoding="utf-8")
        check(
            "G3-lifecycle-1000-requests",
            "PASS" if stress_error is None else "FAIL",
            kind="RealTSF",
            stress=stress_summary,
            indicator=stress_latest,
        )

        # The long stress loop must not recover by stealing focus. If another
        # application became foreground, stop before any MockCOM case.
        assert_owned_fixture_foreground("before MockCOM")
        fixture_command("FocusEditor")

        # The fault matrix uses an explicit fixture command. Lifecycle cases
        # retain and release the real production edit-session callback; the
        # older direct reason cases stay DiagnosticTransport and are not
        # counted as MockProviderLifecycle acceptance.
        mock_faults = {
            "D03-missing-uia": ("missing-uia", 8, "DiagnosticTransport"),
            "D06-password": ("password", 11, "DiagnosticTransport"),
            "D07-unknown-sensitivity": ("unknown-sensitivity", 10, "DiagnosticTransport"),
            "D08-no-layout": ("no-layout", 18, "DiagnosticTransport"),
            "D09-invalid-rect": ("invalid-rect", 20, "DiagnosticTransport"),
            "D10-different-view": ("different-view", 9, "DiagnosticTransport"),
            "D12-focus-race": ("focus-race", 21, "DiagnosticTransport"),
            "D13-request-error": ("request-error", 24, "DiagnosticTransport"),
            "D14-149-151ms": ("late-result", 21, "DiagnosticTransport"),
            "D15-never-delivered": ("never-delivered", 30, "DiagnosticTransport"),
            "D16-late-close": ("late-close", 21, "DiagnosticTransport"),
            "D19-selection": ("selection", 16, "DiagnosticTransport"),
            "D21-reentrancy": ("reentrancy", 29, "DiagnosticTransport"),
            "D22-source-conflict": ("source-conflict", 26, "DiagnosticTransport"),
            "D25-protocol": ("protocol", 24, "DiagnosticTransport"),
            "D26-rollover": ("rollover", 24, "DiagnosticTransport"),
            "D27-reuse": ("reuse", 26, "DiagnosticTransport"),
            "D28-release-cap": ("release-cap", 28, "DiagnosticTransport"),
        }
        for name, (fault, expected_reason, kind) in mock_faults.items():
            try:
                result = fixture_command("Fault|" + fault)
                if result.get("status") != "PASS":
                    raise AssertionError(f"MockCOM fault command was not accepted: {result}")
                before_fault = int(current_oracle["sequence"])
                fixture_command("MoveCaret|0")
                fault_oracle = wait_for(lambda: oracle(before_fault), name + " oracle", timeout=5)
                assert_oracle(fault_oracle)
                value = wait_indicator(
                    lambda candidate: (
                        (candidate.get("tsf") or {}).get("state") == "Unavailable"
                        and (
                            expected_reason is None
                            or (candidate.get("tsf") or {}).get("reason_code") == expected_reason
                        )
                    ),
                    name + " MockCOM diagnostic",
                )
                trace = value.get("tsf") or {}
                if trace.get("sequence", 0) == 0 or value.get("generation") is None:
                    raise AssertionError(f"fault diagnostic lost target identity: {value}")
                if kind == "MockProviderLifecycle":
                    if int(trace.get("final_released") or 0) < 1:
                        raise AssertionError(f"lifecycle callback was not finally released: {trace}")
                    if int(trace.get("callback_entered") or 0) != int(trace.get("callback_completed") or 0):
                        raise AssertionError(f"lifecycle callback entry/completion mismatch: {trace}")
                check(
                    name,
                    "PASS",
                    kind=kind,
                    fault=fault,
                    expected_reason=expected_reason,
                    oracle=fault_oracle,
                    indicator=value,
                )
            except (AssertionError, RuntimeError, ValueError) as error:
                check(name, "FAIL", kind=kind, fault=fault, error=str(error))
            finally:
                try:
                    fixture_command("Fault|clear")
                    fixture_command("FocusEditor")
                except (RuntimeError, OSError):
                    pass

        lifecycle_cases = {
            "N15-lifecycle-never-delivered": {
                "fault": "lifecycle-never-delivered",
                "entered": 0,
                "completed": 0,
                "accepted": 1,
                "request_edit_calls": 1,
                "released": 1,
                "close": True,
            },
            "N16-lifecycle-late-close": {
                "fault": "lifecycle-late-close",
                "entered": 1,
                "completed": 1,
                "accepted": 1,
                "request_edit_calls": 1,
                "released": 1,
                "close": True,
            },
            "N21-lifecycle-reentrancy-close": {
                "fault": "lifecycle-reentrancy-close",
                "entered": 1,
                "completed": 1,
                "accepted": 1,
                "request_edit_calls": 1,
                "released": 1,
                "close": True,
            },
            "N28-lifecycle-release-cap": {
                "fault": "lifecycle-release-cap",
                "entered": 0,
                "completed": 0,
                "accepted": 8,
                "request_edit_calls": 8,
                "released": 8,
                "close": False,
            },
        }
        for name, expected in lifecycle_cases.items():
            try:
                before_value = echo_call({"verb": "input_indicator"})
                before_counters = trace_counters(before_value)
                before_sequence = int(current_oracle["sequence"])
                # Quiesce the observer before arming a lifecycle fault. The
                # next request is then the explicit fault request rather than
                # one ordinary TSF request racing the fault-file update.
                set_indicator_enabled(False)
                fixture_command("Fault|" + expected["fault"])
                set_indicator_enabled(True)
                fixture_command("FocusEditor")
                fixture_command("MoveCaret|0")
                initial = wait_indicator(
                    lambda candidate: (
                        (candidate.get("tsf") or {}).get("state") == "Closed"
                        or (
                            (candidate.get("tsf") or {}).get("state") == "Unavailable"
                            and int((candidate.get("tsf") or {}).get("sequence") or 0) > 0
                        )
                    ),
                    name + " initial terminal phase",
                )
                initial_counters = trace_counters(initial)
                initial_delta = counter_delta(before_counters, initial_counters)
                initial_phase = {
                    "state": (initial.get("tsf") or {}).get("state"),
                    "sequence": (initial.get("tsf") or {}).get("sequence"),
                    "counters": initial_counters,
                }

                # The retained callback is closed/delivered from a later
                # scheduler request. This guarantees the original
                # prepare/invoke/reconcile stack has already returned.
                if expected["fault"] == "lifecycle-release-cap":
                    # Release-cap leaves the observer in an unavailable
                    # state. Recreate the observer before issuing the delayed
                    # drain so the queue is drained by a later scheduler
                    # request even if the original runtime has closed.
                    set_indicator_enabled(False)
                    fixture_command("Fault|clear")
                    fixture_command("Fault|lifecycle-drain")
                    set_indicator_enabled(True)
                    fixture_command("FocusEditor")
                else:
                    fixture_command("Fault|clear")
                    fixture_command("Fault|lifecycle-drain")
                fixture_command("MoveCaret|0")
                drained = wait_indicator(
                    lambda candidate: (
                        trace_counters(candidate)["mock_final_released"]
                        - before_counters["mock_final_released"]
                        >= expected["released"]
                        and trace_counters(candidate)["mock_callback_entered"]
                        - before_counters["mock_callback_entered"]
                        >= expected["entered"]
                    ),
                    name + " delayed drain",
                )
                drained_counters = trace_counters(drained)
                drained_delta = counter_delta(before_counters, drained_counters)
                fixture_command("Fault|clear")

                recovered = drained
                if expected["close"]:
                    fixture_command("FocusEditor")
                    recovered = wait_indicator(
                        lambda candidate: (
                            (candidate.get("sample") or {}).get("geometry", {}).get("source")
                            == "TsfCaret"
                            and trace_counters(candidate)["mock_final_released"]
                            - before_counters["mock_final_released"]
                            >= expected["released"]
                        ),
                        name + " recovered TSF",
                    )
                after_counters = trace_counters(recovered)
                delta = counter_delta(before_counters, after_counters)
                # The initial fault request is the lifecycle count boundary.
                # Close recovery may legitimately create additional ordinary
                # TSF sessions before the final snapshot, so accepted/request
                # counts are asserted at the terminal boundary while callback
                # entry/completion/release counts cover the full case.
                for field, expected_delta in (
                    ("mock_sessions", expected["accepted"]),
                ):
                    if initial_delta[field] != expected_delta:
                        raise AssertionError(
                            f"{field} initial_delta={initial_delta[field]} expected={expected_delta}; before={before_counters} initial={initial_counters} after={after_counters}"
                        )
                if initial_delta["request_edit_calls"] < expected["request_edit_calls"]:
                    raise AssertionError(
                        f"request_edit_calls initial_delta={initial_delta['request_edit_calls']} expected_at_least={expected['request_edit_calls']}; before={before_counters} initial={initial_counters}"
                    )
                if initial_delta["accepted_sessions"] < expected["accepted"]:
                    raise AssertionError(
                        f"accepted_sessions initial_delta={initial_delta['accepted_sessions']} expected_at_least={expected['accepted']}; before={before_counters} initial={initial_counters}"
                    )
                for field, expected_delta in (
                    ("mock_callback_entered", expected["entered"]),
                    ("mock_callback_completed", expected["completed"]),
                    ("mock_final_released", expected["released"]),
                ):
                    if drained_delta[field] != expected_delta:
                        raise AssertionError(
                            f"{field} drained_delta={drained_delta[field]} expected={expected_delta}; before={before_counters} drained={drained_counters} after={after_counters}"
                        )
                created_delta = drained_delta["created_callbacks"]
                released_delta = drained_delta["released_callbacks"]
                final_delta = drained_delta["final_released"]
                if created_delta != released_delta or created_delta != final_delta:
                    raise AssertionError(
                        f"{name} callback create/final-release mismatch created={created_delta} released={released_delta} final={final_delta}; before={before_counters} drained={drained_counters}"
                    )
                if drained_counters["outstanding_callbacks"] != 0:
                    raise AssertionError(f"{name} outstanding callbacks remained after explicit drain: {drained_counters}")
                if drained_counters["pending_callbacks"] != 0:
                    raise AssertionError(f"{name} pending callback remained after explicit drain: {drained_counters}")
                if drained_counters["callback_high_water"] > 8 or drained_counters["pending_high_water"] > 1:
                    raise AssertionError(f"{name} callback high-water cap exceeded: {drained_counters}")
                check(
                    name,
                    "PASS",
                    kind="MockProviderLifecycle",
                    fault=expected["fault"],
                    expected={
                        "accepted_sessions": expected["accepted"],
                        "request_edit_calls": expected["request_edit_calls"],
                        "callback_entered": expected["entered"],
                        "callback_completed": expected["completed"],
                        "final_released": expected["released"],
                    },
                    timeline={
                        "before": before_counters,
                        "initial_terminal": initial_phase,
                        "initial_delta": initial_delta,
                        "drained": drained_counters,
                        "drained_delta": drained_delta,
                        "retained_before_drain": {
                            "created_callbacks": initial_delta["created_callbacks"],
                            "final_released": initial_delta["final_released"],
                            "outstanding_callbacks": initial_counters["outstanding_callbacks"],
                            "pending_callbacks": initial_counters["pending_callbacks"],
                        },
                        "after": after_counters,
                        "delta": delta,
                        "oracle_sequence_before": before_sequence,
                    },
                    indicator=recovered,
                )
            except (AssertionError, RuntimeError, ValueError) as error:
                check(
                    name,
                    "FAIL",
                    kind="MockProviderLifecycle",
                    fault=expected["fault"],
                    error=str(error),
                )
            finally:
                try:
                    fixture_command("Fault|clear")
                    fixture_command("FocusEditor")
                except (RuntimeError, OSError):
                    pass

        if observer_info and not observer_info.get("loaded"):
            capture_observer_identity()
        if observer_info is None:
            check(
                "module-identity",
                "FAIL",
                kind="NativeHWND",
                required=True,
                error="--observer-dll is required for module identity acceptance",
            )
        elif not observer_info.get("modules_enumerated"):
            check(
                "module-identity",
                "FAIL",
                kind="NativeHWND",
                required=True,
                error=observer_info.get("load_error", "module enumeration unavailable"),
            )
        elif not observer_info.get("loaded"):
            check(
                "module-identity",
                "FAIL",
                kind="NativeHWND",
                required=True,
                error="observer DLL digest was not found in the target PID module list",
                observer=observer_info,
            )
        else:
            check(
                "module-identity",
                "PASS",
                kind="NativeHWND",
                required=True,
                observer=observer_info,
            )

        required_manifest = [
            {"name": "N01-owned-fixture-oracle", "evidence_kind": "RealTSF", "required": True},
            {"name": "G2-real-tsf-window-and-coordinate-chain", "evidence_kind": "RealTSF", "required": True},
            {"name": "N02-empty-context", "evidence_kind": "RealTSF", "required": True},
            {"name": "N05-readonly", "evidence_kind": "RealTSF", "required": True},
            {"name": "N11-context-replaced", "evidence_kind": "RealTSF", "required": True},
            {"name": "N12-root-child-two", "evidence_kind": "RealTSF", "required": True},
            {"name": "N13-root-child-one", "evidence_kind": "RealTSF", "required": True},
            {"name": "N20-current-dpi-smoke", "evidence_kind": "DPI", "required": False},
            {"name": "N23-mode-only", "evidence_kind": "RealTSF", "required": True},
            {"name": "N24-geometry-only", "evidence_kind": "RealTSF", "required": True},
            {"name": "N18-repeated-enable-disable", "evidence_kind": "RealTSF", "required": True},
            {"name": "fixture-multiline-real-tsf", "evidence_kind": "RealTSF", "required": True},
            {"name": "G3-lifecycle-1000-requests", "evidence_kind": "RealTSF", "required": True},
            {"name": "N15-lifecycle-never-delivered", "evidence_kind": "MockProviderLifecycle", "required": True},
            {"name": "N16-lifecycle-late-close", "evidence_kind": "MockProviderLifecycle", "required": True},
            {"name": "N21-lifecycle-reentrancy-close", "evidence_kind": "MockProviderLifecycle", "required": True},
            {"name": "N28-lifecycle-release-cap", "evidence_kind": "MockProviderLifecycle", "required": True},
            {"name": "module-identity", "evidence_kind": "NativeHWND", "required": True},
        ]
        by_name = {entry["name"]: entry for entry in checks}
        missing_required = [
            item["name"] for item in required_manifest if item["required"] and item["name"] not in by_name
        ]
        failed_required = [
            item["name"]
            for item in required_manifest
            if item["required"] and by_name.get(item["name"], {}).get("status") == "FAIL"
        ]
        unsupported_required = [
            item["name"]
            for item in required_manifest
            if item["required"] and by_name.get(item["name"], {}).get("status") in ("UNSUPPORTED", "NOT_RUN")
        ]
        optional_unsupported = [
            item["name"]
            for item in required_manifest
            if not item["required"] and by_name.get(item["name"], {}).get("status") in ("UNSUPPORTED", "NOT_RUN")
        ]
        if failed_required:
            overall = "FAIL"
        elif missing_required or unsupported_required:
            overall = "PARTIAL"
        else:
            overall = "PASS"
        results = {
            "provider": args.provider,
            "overall": overall,
            "fixture": {"path": str(fixture), "pid": fixture_process.pid, "hwnd": ready["hwnd"]},
            "echo": {"path": str(root / "echo-acceptance.exe"), "sha256": sha256(root / "echo-acceptance.exe")},
            "observer_dll": observer_info,
            "required_manifest": required_manifest,
            "required_summary": {
                "missing": missing_required,
                "failed": failed_required,
                "unsupported": unsupported_required,
                "optional_unsupported": optional_unsupported,
            },
            "evidence_summary": {
                "real_tsf": sum(1 for entry in checks if entry.get("kind") == "RealTSF"),
                "mock_provider_lifecycle": sum(
                    1 for entry in checks if entry.get("kind") == "MockProviderLifecycle"
                ),
                "diagnostic_transport": sum(
                    1 for entry in checks if entry.get("kind") == "DiagnosticTransport"
                ),
            },
            "checks": checks,
        }
        (root / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        return 0 if overall == "PASS" else 2
    except (AssertionError, RuntimeError, ValueError, OSError, subprocess.SubprocessError) as error:
        # Preserve a machine-readable safety abort even when the run stops
        # before the normal manifest can be assembled. In particular, a lost
        # fixture foreground must never be recovered by activating another
        # window or silently downgraded to a partial result.
        try:
            (root / "runner-error.json").write_text(
                json.dumps(
                    {
                        "schema": "echo.caret.runner-error.v1",
                        "overall": "FAIL",
                        "error": str(error),
                        "checks": checks,
                        "foreground_policy": "fail-closed; no recovery activation after Echo start",
                    },
                    indent=2,
                ),
                encoding="utf-8",
            )
        except OSError:
            pass
        raise
    finally:
        try:
            if ready is not None:
                nonce = uuid.uuid4().hex
                command_path = root / ready.get("command", "fixture.command.json")
                response_path = root / ready.get("response", "fixture.response.json")
                response_path.unlink(missing_ok=True)
                temp = command_path.with_suffix(".tmp")
                temp.write_text(nonce + "|ShutdownOwnedFixture", encoding="utf-8")
                os.replace(temp, command_path)
                wait_for(lambda: read_json(response_path), "fixture shutdown response", timeout=2)
        except (OSError, ValueError, RuntimeError):
            pass
        for child in children:
            try:
                child.terminate()
                child.wait(timeout=3)
            except Exception:
                try:
                    child.kill()
                except Exception:
                    pass
        for handle in (echo_log, fixture_log):
            if handle is not None:
                handle.close()


if __name__ == "__main__":
    raise SystemExit(main())
