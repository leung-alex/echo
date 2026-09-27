"""Warp passive-badge suppression and explicit popup placement acceptance. No text/clipboard mutation.

Requires an already running Warp. Only focus, pointer movement, Alt+V and Escape
are used; all Echo state is isolated synthetic data. The test accepts either a
real caret or Warp's bounded input-lane estimate and checks badge stability
without mouse following.
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
    u = ctypes.WinDLL('user32', use_last_error=True)
    pointer = ctypes.c_void_p
    u.GetForegroundWindow.restype = pointer
    u.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
    u.FindWindowW.restype = pointer
    u.SetForegroundWindow.argtypes = [pointer]
    u.GetWindowThreadProcessId.argtypes = [pointer, ctypes.POINTER(ctypes.c_ulong)]
    sequence = 0
    children, checks = [], []

    # Bind the isolated observer to the exact Warp root before starting Echo.
    # ECHO_NATIVE_TEST_ROOT enables the bridge and the observer's fail-closed
    # target guard, so both target identities must be present at first capture.
    u.IsWindowVisible.argtypes = [pointer]
    u.GetClassNameW.argtypes = [pointer, ctypes.c_wchar_p, ctypes.c_int]
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
    env = dict(
        os.environ,
        ECHO_DATA_DIR=str(root / 'data'),
        ECHO_NATIVE_TEST_ROOT=str(root),
        ECHO_NATIVE_TEST_TARGET_PID=str(warp_pid),
        ECHO_NATIVE_TEST_TARGET_HWND=str(warp_hwnd),
    )

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

    def expected_badge(sample):
        geometry = sample['geometry']
        target = geometry['rect']
        work = geometry['work_area']
        scale = lambda dip: max(1, round(dip * int(geometry['dpi'] or 96) / 96.0))
        width, height, gap = scale(48), scale(36), scale(8)
        work_x, work_y, work_width, work_height = map(int, work)
        target_x, target_y, target_width, target_height = map(int, target)
        right, bottom = work_x + work_width, work_y + work_height
        x = target_x + target_width + gap
        if x + width > right:
            x = target_x - gap - width
        y = target_y - gap - height
        if y < work_y:
            y = target_y + target_height + gap
        return [
            max(work_x, min(x, right - width)),
            max(work_y, min(y, bottom - height)),
            width,
            height,
        ]

    with open(root / 'echo.log', 'w', encoding='utf-8') as log:
        try:
            echo = subprocess.Popen([str(executable), '--background'], env=env, stdout=log, stderr=log,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
            children.append(echo)
            wait(lambda: (root / 'native-control').exists(), 'test bridge')
            u.ShowWindow.argtypes = [pointer, ctypes.c_int]
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
            first = call('input_indicator')
            deadline = time.monotonic() + 2
            while not first['visible'] and time.monotonic() < deadline:
                time.sleep(.08)
                first = call('input_indicator')
            exact_sources = {'NativeCaret', 'UiaCaret', 'MsaaCaret', 'TsfCaret'}
            accepted_sources = exact_sources | {'EditorLeadingEdge'}
            def visible_badge():
                value = call('input_indicator')
                return value if value['visible'] else None
            badge_expected = False
            if first['visible']:
                sample = first.get('sample') or {}
                geometry = sample.get('geometry') or {}
                assert sample.get('window') == hwnd, first
                assert geometry.get('source') in accepted_sources, first
                assert first['label'] in ('中', 'EN'), first
                assert u.GetForegroundWindow() == hwnd
                expected = expected_badge(sample)
                assert all(abs(int(actual) - int(wanted)) <= 2
                           for actual, wanted in zip(first['rect'], expected)), (first, expected)
                check('warp-badge-uses-exact-caret-or-stable-input-lane', first)
                badge_expected = True
                stable_rect = first['rect']
                u.SetCursorPos(host.left + 700, host.top + 500)
                time.sleep(1)
                second = wait(visible_badge, 'Warp caret badge after pointer movement')
                assert (second.get('sample') or {}).get('geometry', {}).get('source') in accepted_sources, second
                assert second['rect'] == stable_rect, (stable_rect, second)
                assert u.GetForegroundWindow() == hwnd
                check('warp-badge-does-not-follow-mouse', second)
            else:
                u.SetCursorPos(host.left + 700, host.top + 500)
                deadline = time.monotonic() + 2
                second = call('input_indicator')
                while not second['visible'] and time.monotonic() < deadline:
                    time.sleep(.08)
                    second = call('input_indicator')
                if second['visible']:
                    sample = second.get('sample') or {}
                    geometry = sample.get('geometry') or {}
                    assert sample.get('window') == hwnd, second
                    assert geometry.get('source') in accepted_sources, second
                    assert second['label'] in ('中', 'EN'), second
                    assert u.GetForegroundWindow() == hwnd
                    expected = expected_badge(sample)
                    assert all(abs(int(actual) - int(wanted)) <= 2
                               for actual, wanted in zip(second['rect'], expected)), (second, expected)
                    check('warp-badge-uses-exact-caret-or-stable-input-lane', second)
                    badge_expected = True
                    stable_rect = second['rect']
                    u.SetCursorPos(host.left + 200, host.top + 200)
                    time.sleep(1)
                    third = wait(visible_badge, 'Warp caret badge after provider bootstrap')
                    assert (third.get('sample') or {}).get('geometry', {}).get('source') in accepted_sources, third
                    assert third['rect'] == stable_rect, (stable_rect, third)
                    assert u.GetForegroundWindow() == hwnd
                    check('warp-badge-does-not-follow-mouse', third)
                else:
                    check('warp-no-passive-pointer-fallback', first)
                    assert u.GetForegroundWindow() == hwnd
                    check('warp-no-mouse-following', second)
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
            wait(lambda: not call('input_indicator')['visible'], 'badge stays hidden during Quick Insert')
            call('capture', file='warp-popup.png')
            check('warp-popup-uses-captured-pointer-and-suppresses-badge', state['quick_insert'])
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
            if badge_expected:
                resumed = wait(lambda: (s if (s := call('input_indicator'))['visible'] else None),
                               'badge resumes after Esc')
                assert (resumed.get('sample') or {}).get('geometry', {}).get('source') in accepted_sources, resumed
                check('warp-badge-resumes-after-esc', resumed)
            else:
                resumed = call('input_indicator')
                assert not resumed['visible'], resumed
                check('warp-badge-remains-hidden-after-esc', resumed)
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
