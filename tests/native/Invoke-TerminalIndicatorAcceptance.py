"""Owned console badge and plain-paste Esc regression; no clipboard mutation."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    assert os.environ.get('ECHO_WINDOWS_ACCEPTANCE') == '1'
    root = args.evidence.resolve()
    root.mkdir(parents=True, exist_ok=False)
    (root / 'data').mkdir()
    (root / 'data/synthetic-fixture.json').write_text(json.dumps(dict(synthetic=True, capture_enabled=False)))
    executable = root / 'echo-terminal-test.exe'
    shutil.copy2(args.executable, executable)
    env = dict(os.environ, ECHO_DATA_DIR=str(root / 'data'), ECHO_NATIVE_TEST_ROOT=str(root))
    u = ctypes.WinDLL('user32', use_last_error=True)
    pointer = ctypes.c_void_p
    u.GetForegroundWindow.restype = pointer
    u.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
    u.FindWindowW.restype = pointer
    u.SetForegroundWindow.argtypes = [pointer]
    u.GetWindowThreadProcessId.argtypes = [pointer, ctypes.POINTER(ctypes.c_ulong)]
    sequence = 0
    children, checks = [], []

    def wait(probe, label, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            value = probe()
            if value:
                return value
            time.sleep(.04)
        raise AssertionError(label)

    def call(verb):
        nonlocal sequence
        sequence += 1
        request = root / 'native-control/request.json'
        temporary = request.with_suffix('.tmp')
        temporary.write_text(json.dumps(dict(id=str(sequence), pid=echo.pid, verb=verb)))
        os.replace(temporary, request)

        def response():
            try:
                value = json.loads((root / 'native-control/response.json').read_text(encoding='utf-8-sig'))
                return value if value['id'] == str(sequence) else None
            except (OSError, ValueError):
                return None
        value = wait(response, verb)
        assert value['status'] == 'PASS', value
        return value['value']

    def key(vk):
        u.keybd_event(vk, 0, 0, 0)
        u.keybd_event(vk, 0, 2, 0)

    def activate(hwnd):
        owner = ctypes.c_ulong()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        assert owner.value in [p.pid for p in children], 'unowned foreground target'
        if u.GetForegroundWindow() != hwnd:
            key(0x12)
            u.SetForegroundWindow(hwnd)
        wait(lambda: u.GetForegroundWindow() == hwnd, 'owned foreground')

    def check(name, details):
        checks.append(dict(name=name, status='PASS', details=details))
        (root / 'checks.json').write_text(json.dumps(checks, ensure_ascii=False, indent=2), encoding='utf-8')
        print('PASS', name, flush=True)

    with open(root / 'echo.log', 'w', encoding='utf-8') as log:
        try:
            echo = subprocess.Popen([str(executable), '--background'], env=env, stdout=log, stderr=log,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
            children.append(echo)
            wait(lambda: (root / 'native-control').exists(), 'test bridge')
            title = 'Echo isolated terminal indicator ' + root.name
            terminal = subprocess.Popen(['pwsh', '-NoProfile', '-Command',
                                         "[Console]::Title='" + title + "'; Start-Sleep -Seconds 90"],
                                        creationflags=subprocess.CREATE_NEW_CONSOLE)
            children.append(terminal)
            hwnd = wait(lambda: u.FindWindowW('ConsoleWindowClass', title), 'owned console')
            activate(hwnd)
            def badge():
                value = call('input_indicator')
                return value if value['visible'] and value['sample']['window'] == hwnd else None
            first = wait(badge, 'console badge')
            assert first['label'] in ('中', 'EN') and u.GetForegroundWindow() == hwnd
            check('console-caret-and-mode-visible-without-taking-focus', first)
            time.sleep(2)
            second = wait(badge, 'stable console badge')
            assert second['generation'] == first['generation'], 'self-induced focus invalidation loop'
            assert second['counts'][0] - first['counts'][0] < 60, 'unbounded polling'
            check('console-observer-stays-stable', second)
            assert u.GetForegroundWindow() == hwnd
            u.keybd_event(0x12, 0, 0, 0)
            key(0x56)
            u.keybd_event(0x12, 0, 2, 0)
            wait(lambda: call('metrics')['visible'], 'plain-paste popup')
            assert u.GetForegroundWindow() == hwnd
            key(27)
            wait(lambda: not call('metrics')['visible'], 'Esc hides plain-paste popup')
            check('plain-paste-escape-hides-popup', {'foreground_preserved': u.GetForegroundWindow() == hwnd})
            u.keybd_event(0x12, 0, 0, 0)
            key(0x56)
            u.keybd_event(0x12, 0, 2, 0)
            wait(lambda: call('metrics')['visible'], 'second plain-paste popup')
            acknowledgement = call('plain_paste_ack_fixture')
            wait(lambda: not call('metrics')['visible'] and call('metrics')['inline']['readiness'][0] == 0,
                 'synthetic completion retires keyboard lease')
            check('synthetic-paste-ack-retires-keyboard-lease', acknowledgement)
            subprocess.run([str(executable), '--settings'], env=env, check=True, timeout=10,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            state = wait(lambda: (s if (s := call('metrics'))['visible'] and s['route'] == 'settings' else None), 'settings')
            manager = wait(lambda: u.FindWindowW(None, 'Echo Recall'), 'settings HWND')
            activate(manager)
            key(27)
            wait(lambda: not call('metrics')['visible'], 'Esc hides settings directly')
            check('settings-escape-hides-to-tray', {'direct_hide': True})
        finally:
            subprocess.run([str(executable), '--quit'], env=env, timeout=10,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=10)


if __name__ == '__main__':
    main()
