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

    def fixture_command(command):
        if ready is None:
            raise RuntimeError("fixture is not ready")
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
        entry = {"name": name, "status": status}
        entry.update(details)
        checks.append(entry)
        (root / "checks.json").write_text(json.dumps(checks, indent=2), encoding="utf-8")
        print(status, name, flush=True)
        return entry

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
        required = ("pid", "hwnd", "input_hwnd", "visible", "dpi", "window_rect", "caret", "sequence")
        if any(key not in value for key in required):
            raise AssertionError(f"oracle missing fields: {value}")
        if value["pid"] != fixture_process.pid or int(value["hwnd"]) != int(ready["hwnd"]):
            raise AssertionError(f"oracle ownership mismatch: {value}")
        input_hwnd = int(value["input_hwnd"])
        root_hwnd = int(value["hwnd"])
        if not value["visible"] or not input_hwnd:
            raise AssertionError(f"oracle HWND is not a visible owned target: {value}")
        if input_hwnd != root_hwnd:
            if int(user32.GetAncestor(ctypes.c_void_p(input_hwnd), 2)) != root_hwnd:
                raise AssertionError(f"oracle child HWND is outside root: {value}")
            owner = ctypes.c_uint32()
            if user32.GetWindowThreadProcessId(ctypes.c_void_p(input_hwnd), ctypes.byref(owner)) == 0 or owner.value != fixture_process.pid:
                raise AssertionError(f"oracle child HWND owner mismatch: {value}")
        if len(value["caret"]) != 4 or value["caret"][2] <= value["caret"][0] or value["caret"][3] <= value["caret"][1]:
            raise AssertionError(f"oracle caret is invalid: {value}")

    try:
        echo_log = (root / "echo.log").open("w", encoding="utf-8")
        echo = subprocess.Popen([str(root / "echo-acceptance.exe"), "--background"], env=env,
                                stdout=echo_log, stderr=echo_log, creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(echo)
        wait_for(lambda: (root / "native-control").is_dir(), "Echo native bridge", timeout=12)
        fixture_log = (root / "fixture.log").open("w", encoding="utf-8")
        fixture_process = subprocess.Popen([str(fixture), str(root)], env=env,
                                           stdout=fixture_log, stderr=fixture_log,
                                           creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(fixture_process)
        wait_for(lambda: (root / "fixture.ready.json").exists(), "owned WPF fixture", timeout=12)
        ready = read_json(root / "fixture.ready.json")
        if int(ready["pid"]) != fixture_process.pid:
            raise RuntimeError(f"fixture PID changed: {ready}")
        if int(user32.GetForegroundWindow()) != int(ready["hwnd"]):
            # Windows foreground-lock policy permits the owned fixture to be
            # activated after a bounded Alt tap. This does not send text.
            user32.keybd_event(0x12, 0, 0, None)
            user32.keybd_event(0x12, 0, 2, None)
            user32.SetForegroundWindow(ctypes.c_void_p(ready["hwnd"]))
        wait_for(lambda: int(user32.GetForegroundWindow()) == int(ready["hwnd"]), "owned fixture foreground", timeout=5)

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

        scenarios = [
            ("N02-empty-context", "CreateScenario|empty", True),
            ("N05-readonly", "CreateScenario|readonly", False),
            ("N11-context-replaced", "CreateScenario|context-replaced", True),
            ("N20-current-dpi-smoke", "CreateScenario|mixed-dpi", True),
            ("N23-mode-only", "SetSyntheticModeForFixture|normal", True),
            ("N24-geometry-only", "MoveCaret|0", True),
        ]
        last_epoch = (initial.get("tsf") or {}).get("context_epoch")
        for name, command, require_ready in scenarios:
            before = int(current_oracle["sequence"])
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
                    validate_ready(name, current_oracle, value, last_epoch if name == "N11-context-replaced" else None)
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

        # G3 stress: sample the bridge until the target-owned production
        # acquisition counter advances by 1000. RPC reads are only observers;
        # they are not counted as TSF requests.
        stress_count = 1000
        stress_started = time.monotonic()
        stress_before = echo_call({"verb": "input_indicator"})
        stress_before_tsf = stress_before.get("tsf") or {}
        acquisition_start = int(stress_before_tsf.get("api_requests") or 0)
        max_pending = 0
        max_outstanding = 0
        max_high_water = 0
        callback_created = 0
        callback_released = 0
        stress_latest = stress_before
        stress_error = None
        reads = 0
        try:
            while True:
                stress_latest = echo_call({"verb": "input_indicator"})
                reads += 1
                trace = stress_latest.get("tsf") or {}
                max_pending = max(max_pending, int(trace.get("pending_callbacks") or 0))
                max_outstanding = max(max_outstanding, int(trace.get("outstanding_callbacks") or 0))
                max_high_water = max(max_high_water, int(trace.get("callback_high_water") or 0))
                callback_created = max(callback_created, int(trace.get("created_callbacks") or 0))
                callback_released = max(callback_released, int(trace.get("released_callbacks") or 0))
                if max_pending > 8 or max_outstanding > 8 or max_high_water > 8:
                    raise AssertionError(
                        f"callback cap exceeded pending={max_pending} outstanding={max_outstanding} high_water={max_high_water}"
                    )
                acquisition_delta = int(trace.get("api_requests") or 0) - acquisition_start
                if acquisition_delta >= stress_count:
                    break
                if time.monotonic() - stress_started > 120:
                    raise AssertionError(
                        f"target acquisition timeout delta={acquisition_delta} reads={reads}"
                    )
            if callback_created != callback_released:
                raise AssertionError(
                    f"callback create/release mismatch created={callback_created} released={callback_released}"
                )
        except (AssertionError, RuntimeError, ValueError) as error:
            stress_error = str(error)
        stress_summary = {
            "requested_acquisitions": stress_count,
            "observed_acquisitions": int((stress_latest.get("tsf") or {}).get("api_requests") or 0) - acquisition_start,
            "observation_reads": reads,
            "elapsed_ms": round((time.monotonic() - stress_started) * 1000),
            "before_counts": stress_before.get("counts"),
            "after_counts": stress_latest.get("counts"),
            "max_pending_callbacks": max_pending,
            "max_outstanding_callbacks": max_outstanding,
            "max_callback_high_water": max_high_water,
            "callback_created": callback_created,
            "callback_released": callback_released,
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

        # The long stress loop can lose the foreground to an unrelated desktop
        # window. Revalidate the owned HWND before starting MockCOM faults so a
        # watchdog close is reported as a focus case rather than attributed to
        # the adapter.
        user32.keybd_event(0x12, 0, 0, None)
        user32.keybd_event(0x12, 0, 2, None)
        user32.SetForegroundWindow(ctypes.c_void_p(ready["hwnd"]))
        wait_for(
            lambda: int(user32.GetForegroundWindow()) == int(ready["hwnd"]),
            "owned fixture foreground before MockCOM",
            timeout=5,
        )
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
            "N15-lifecycle-never-delivered": ("lifecycle-never-delivered", None, "MockProviderLifecycle"),
            "N16-lifecycle-late-close": ("lifecycle-late-close", None, "MockProviderLifecycle"),
            "N21-lifecycle-reentrancy-close": ("lifecycle-reentrancy-close", None, "MockProviderLifecycle"),
            "N28-lifecycle-release-cap": ("lifecycle-release-cap", 28, "MockProviderLifecycle"),
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

        if observer_info:
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
            except OSError as error:
                observer_info["modules_enumerated"] = False
                observer_info["loaded"] = "UNAVAILABLE"
                observer_info["load_error"] = str(error)
        statuses = [entry["status"] for entry in checks]
        has_diagnostic_transport = any(
            entry.get("kind") == "DiagnosticTransport" for entry in checks
        )
        overall = (
            "PASS"
            if statuses and all(status == "PASS" for status in statuses) and not has_diagnostic_transport
            else "PARTIAL"
        )
        results = {
            "provider": args.provider,
            "overall": overall,
            "fixture": {"path": str(fixture), "pid": fixture_process.pid, "hwnd": ready["hwnd"]},
            "echo": {"path": str(root / "echo-acceptance.exe"), "sha256": sha256(root / "echo-acceptance.exe")},
            "observer_dll": observer_info,
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
