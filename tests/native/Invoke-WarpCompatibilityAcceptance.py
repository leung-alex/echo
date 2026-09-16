"""Warp pointer-status and popup placement acceptance. No text/clipboard mutation.

Requires an already running Warp. Only focus, pointer movement, Alt+V and Escape
are used; all Echo state is isolated synthetic data. Does not certify caret following.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import psutil


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

    def call(verb, **kwargs):
        nonlocal sequence
        sequence += 1
        request = root / 'native-control/request.json'
        temporary = request.with_suffix('.tmp')
        temporary.write_text(json.dumps(dict(id=str(sequence), pid=echo.pid, verb=verb, **kwargs)))
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
        assert owner.value in [p.pid for p in children] or (owner.value == warp_pid and hwnd == warp_hwnd), 'unexpected foreground target'
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
            u.IsWindowVisible.argtypes = [pointer]
            u.GetClassNameW.argtypes = [pointer, ctypes.c_wchar_p, ctypes.c_int]
            u.ShowWindow.argtypes = [pointer, ctypes.c_int]
            rows = []
            @ctypes.WINFUNCTYPE(ctypes.c_int, pointer, pointer)
            def visit(h, _):
                pid = ctypes.c_ulong()
                u.GetWindowThreadProcessId(h, ctypes.byref(pid))
                cls = ctypes.create_unicode_buffer(128)
                u.GetClassNameW(h, cls, 128)
                try:
                    if cls.value == 'Window Class' and u.IsWindowVisible(h) and psutil.Process(pid.value).name().lower() == 'warp.exe':
                        rows.append((h, pid.value))
                except psutil.Error:
                    pass
                return 1
            u.EnumWindows(visit, 0)
            assert rows, 'NOT_RUN: Warp is not running'
            warp_hwnd, warp_pid = rows[0]
            hwnd = warp_hwnd
            u.ShowWindow(hwnd, 9)
            class Rect(ctypes.Structure):
                _fields_ = [(n, ctypes.c_long) for n in ['left', 'top', 'right', 'bottom']]
            u.GetWindowRect.argtypes = [pointer, ctypes.POINTER(Rect)]
            u.GetDpiForWindow.argtypes = [pointer]
            host = Rect()
            assert u.GetWindowRect(hwnd, ctypes.byref(host))
            activate(hwnd)
            pointer_x, pointer_y = host.left + 200, host.top + 200
            u.SetCursorPos(pointer_x, pointer_y)
            def badge():
                value = call('input_indicator')
                return value if value['visible'] and value['sample']['window'] == hwnd else None
            first = wait(badge, 'Warp pointer badge')
            assert first['label'] in ('中', 'EN') and u.GetForegroundWindow() == hwnd
            scale = u.GetDpiForWindow(hwnd) / 96
            expected = [pointer_x + 1 + round(8 * scale), pointer_y - round(44 * scale), round(48 * scale), round(36 * scale)]
            first = wait(lambda: (v if (v := badge()) and v['rect'] == expected else None), 'badge follows pointer')
            check('warp-pointer-mode-without-taking-focus', first)
            time.sleep(.15)
            call('input_indicator_capture', file='warp-badge.png')
            time.sleep(2)
            second = wait(badge, 'stable Warp badge')
            assert second['generation'] == first['generation'], 'self-induced focus invalidation loop'
            assert second['counts'][0] - first['counts'][0] < 60, 'unbounded polling'
            check('warp-observer-stays-stable', second)
            assert u.GetForegroundWindow() == hwnd
            pointer_x, pointer_y = host.left + 200, host.top + 200
            u.SetCursorPos(pointer_x, pointer_y)
            u.keybd_event(0x12, 0, 0, 0)
            key(0x56)
            u.keybd_event(0x12, 0, 2, 0)
            wait(lambda: call('metrics')['visible'], 'plain-paste popup')
            state = call('metrics')
            assert state['quick_insert']['anchor_source'] == 'pointer-fallback', state['quick_insert']
            popup = u.FindWindowW(None, 'Echo Recall')
            popup_owner = ctypes.c_ulong()
            u.GetWindowThreadProcessId(popup, ctypes.byref(popup_owner))
            assert popup_owner.value == echo.pid, 'wrong popup identity'
            popup_rect = Rect()
            assert u.GetWindowRect(popup, ctypes.byref(popup_rect))
            card_x = popup_rect.left + state['panel'][0] * state['scale_factor']
            assert abs(card_x - pointer_x) <= 2, (card_x, pointer_x)
            wait(lambda: not call('input_indicator')['visible'], 'badge hides during Quick Insert')
            call('capture', file='warp-popup.png')
            check('warp-popup-uses-pointer-and-suppresses-badge', state['quick_insert'])
            assert u.GetForegroundWindow() == hwnd
            initial_space = state['space']
            key(9)
            wait(lambda: call('metrics')['space'] != initial_space, 'Tab switches Echo space')
            u.keybd_event(0x10, 0, 0, 0)
            key(9)
            u.keybd_event(0x10, 0, 2, 0)
            wait(lambda: call('metrics')['space'] == initial_space, 'Shift+Tab returns Echo space')
            for vk in [0x26, 0x28]:
                key(vk)
                wait(lambda: any(e['kind'] == 'plain-navigation-owned' and e['detail'] == vk
                                 for e in call('metrics')['inline_trace']), 'arrow consumed by Echo hook')
            check('plain-paste-tab-shift-tab-and-arrows-owned-by-echo', {'space': initial_space})
            key(27)
            wait(lambda: not call('metrics')['visible'], 'Esc hides plain-paste popup')
            check('plain-paste-escape-hides-popup', {'foreground_preserved': u.GetForegroundWindow() == hwnd})
            wait(badge, 'badge resumes after Esc')
            subprocess.run([str(executable), '--settings'], env=env, check=True, timeout=10,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            state = wait(lambda: (s if (s := call('metrics'))['visible'] and s['route'] == 'settings' else None), 'settings')
            manager = wait(lambda: u.FindWindowW(None, 'Echo Recall'), 'settings HWND')
            activate(manager)
            wait(lambda: not call('input_indicator')['visible'], 'badge hides on focus loss')
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
