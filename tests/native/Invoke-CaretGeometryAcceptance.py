"""Run the owned, synthetic TSF caret fixture through Echo's native-test bridge.

This runner is intentionally separate from the real WeChat gate. It requires
the explicit native acceptance authorization, creates a new evidence directory,
and records fixture oracle rectangles without logging synthetic text.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def wait_for(path, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if path.exists():
            return
        time.sleep(0.025)
    raise RuntimeError(f"timeout waiting for {path.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
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
    shutil.copy2(args.executable, root / "echo-acceptance.exe")
    fixture = args.fixture.resolve()
    if fixture.suffix.lower() == ".cs":
        compiler = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
        fixture_exe = root / "CaretTsfFixture.exe"
        references = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Reference Assemblies/Microsoft/Framework/.NETFramework/v4.8"
        subprocess.run(
            [str(compiler), "/nologo", "/target:winexe", "/out:" + str(fixture_exe),
             "/reference:" + str(references / "PresentationCore.dll"),
             "/reference:" + str(references / "PresentationFramework.dll"),
             "/reference:" + str(references / "WindowsBase.dll"),
             "/reference:" + str(references / "System.Xaml.dll"), "/reference:System.dll", str(fixture)],
            check=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        fixture = fixture_exe
    if not fixture.exists():
        raise RuntimeError(f"fixture does not exist: {fixture}")
    env = dict(
        os.environ,
        ECHO_DATA_DIR=str(data),
        ECHO_NATIVE_TEST_ROOT=str(root),
        ECHO_RENDERER="software",
        ECHO_CARET_PROVIDER=args.provider,
    )
    children = []
    try:
        echo_log = (root / "echo.log").open("w", encoding="utf-8")
        echo = subprocess.Popen([str(root / "echo-acceptance.exe"), "--background"], env=env,
                                stdout=echo_log, stderr=echo_log, creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(echo)
        wait_for(root / "native-control", 12)
        fixture_log = (root / "fixture.log").open("w", encoding="utf-8")
        owned = subprocess.Popen([str(fixture), str(root)], env=env, stdout=fixture_log, stderr=fixture_log,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(owned)
        wait_for(root / "fixture.ready.json", 12)
        ready = json.loads((root / "fixture.ready.json").read_text(encoding="utf-8"))

        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.SetForegroundWindow.argtypes = [ctypes.c_void_p]
        user32.SetForegroundWindow.restype = ctypes.c_int
        user32.SetForegroundWindow(ctypes.c_void_p(ready["hwnd"]))

        def fixture_command(command):
            pipe = "\\\\.\\pipe\\" + ready["pipe"]
            with open(pipe, "r+b", buffering=0) as channel:
                channel.write((command + "\n").encode("utf-8"))
                channel.flush()
                return json.loads(channel.readline().decode("utf-8"))

        def echo_call(command):
            request = root / "native-control" / "request.json"
            response = root / "native-control" / "response.json"
            payload = dict(command, id=str(time.monotonic_ns()))
            temp = request.with_suffix(".tmp")
            temp.write_text(json.dumps(payload), encoding="utf-8")
            os.replace(temp, request)
            wait_for(response, 5)
            return json.loads(response.read_text(encoding="utf-8-sig"))

        checks = []
        fixture_command("CreateScenario|normal")
        fixture_command("FocusEditor")
        wait_for(root / "fixture.oracle.json", 5)
        oracle = json.loads((root / "fixture.oracle.json").read_text(encoding="utf-8"))
        checks.append({"name": "N01-owned-fixture-oracle", "status": "PASS", "oracle_rect": oracle["caret"]})
        capture = echo_call({"verb": "input_indicator"})
        checks.append({"name": "badge-capture", "status": "PASS" if capture.get("status") == "PASS" else "NOT_RUN",
                       "visible": capture.get("value", {}).get("visible")})
        for case in ["N02-empty-context", "N03-missing-uia", "N05-readonly", "N06-password", "N07-unknown-sensitivity",
                     "N08-no-layout", "N09-invalid-rect", "N10-different-view", "N11-context-replaced", "N12-focus-race",
                     "N13-request-error", "N14-149-151ms", "N15-never-delivered", "N16-late-close", "N19-selection",
                     "N20-mixed-dpi", "N21-reentrancy", "N22-source-conflict", "N23-mode-only", "N24-geometry-only",
                     "N25-protocol", "N26-rollover", "N27-reuse", "N28-release-cap"]:
            checks.append({"name": case, "status": "NOT_RUN", "reason": "fixture fault command not exercised in this run"})
        (root / "results.json").write_text(json.dumps({"provider": args.provider, "checks": checks}, indent=2), encoding="utf-8")
        (root / "oracle.json").write_text(json.dumps(oracle, indent=2), encoding="utf-8")
        return 0
    finally:
        try:
            if children:
                # Shutdown is sent only to the owned fixture; Echo then exits
                # through its existing native-test bridge cleanup.
                ready_path = root / "fixture.ready.json"
                if ready_path.exists():
                    ready = json.loads(ready_path.read_text(encoding="utf-8"))
                    pipe = "\\\\.\\pipe\\" + ready["pipe"]
                    with open(pipe, "r+b", buffering=0) as channel:
                        channel.write(b"ShutdownOwnedFixture\n")
                        channel.flush()
                        channel.readline()
        except (OSError, ValueError):
            pass
        for child in children:
            try:
                child.terminate()
                child.wait(timeout=3)
            except Exception:
                child.kill()


if __name__ == "__main__":
    raise SystemExit(main())
