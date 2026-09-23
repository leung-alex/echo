"""Owned synthetic input-mode acceptance; never reads or edits user documents.

Automated IMM changes validate state plumbing, not physical IME compatibility.
"""
import argparse
import base64
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import psutil
import winreg


def installed_profiles():
    base = r'SOFTWARE\Microsoft\CTF\TIP'
    result = []
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, base) as tips:
        for index in range(winreg.QueryInfoKey(tips)[0]):
            clsid = winreg.EnumKey(tips, index)
            try:
                with winreg.OpenKey(tips, clsid + r'\LanguageProfile\0x00000804') as profiles:
                    for n in range(winreg.QueryInfoKey(profiles)[0]):
                        profile = winreg.EnumKey(profiles, n)
                        with winreg.OpenKey(profiles, profile) as key:
                            name = winreg.QueryValueEx(key, 'Description')[0]
                            if name in ('Microsoft Pinyin', '豆包输入法'):
                                result.append((name, clsid, profile))
            except OSError:
                continue
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('ECHO_WINDOWS_ACCEPTANCE') != '1':
        raise RuntimeError('Explicit native acceptance authorization required')
    root = args.evidence.resolve()
    root.mkdir(parents=True, exist_ok=False)
    (root / 'data').mkdir()
    (root / 'data/synthetic-fixture.json').write_text(json.dumps(dict(synthetic=True, capture_enabled=False)))
    executable = root / 'echo-acceptance.exe'
    shutil.copy2(args.executable, executable)
    repo = Path(__file__).resolve().parents[2]
    compiler = Path(os.environ['WINDIR']) / 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    fixture_exe = root / 'InputFixture.exe'
    subprocess.run([str(compiler), '/nologo', '/target:exe', '/out:' + str(fixture_exe),
                    '/reference:System.Windows.Forms.dll', '/reference:System.Drawing.dll',
                    '/reference:System.Web.Extensions.dll', str(repo / 'tests/native/EchoInlineFixture.cs')],
                   check=True, creationflags=subprocess.CREATE_NO_WINDOW)
    env = dict(os.environ, ECHO_DATA_DIR=str(root / 'data'), ECHO_NATIVE_TEST_ROOT=str(root), ECHO_RENDERER='software',
               ECHO_INDICATOR_FRAME_TRACE='1')
    logs, children, checks = [], [], []
    sequence = 0
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    user32.GetWindowLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int]
    user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t

    def start(exe, *argv):
        log = open(root / (exe.stem + '.log'), 'w', encoding='utf-8')
        logs.append(log)
        child = subprocess.Popen([str(exe), *argv], env=env, stdout=log, stderr=log,
                                 creationflags=subprocess.CREATE_NO_WINDOW)
        children.append(child)
        return child

    def wait(probe, label, timeout=8):
        end = time.monotonic() + timeout
        last = None
        while time.monotonic() < end:
            last = probe()
            if last:
                return last
            time.sleep(.025)
        raise AssertionError(f'{label}: {last}')

    def call(fixture=False, **command):
        nonlocal sequence
        sequence += 1
        command['id'] = str(sequence)
        if not fixture:
            command['pid'] = echo.pid
        request = root / ('native-command.json' if fixture else 'native-control/request.json')
        response = root / ('native-response.json' if fixture else 'native-control/response.json')
        request.parent.mkdir(exist_ok=True)
        temp = request.with_suffix('.tmp')
        temp.write_text(json.dumps(command), encoding='utf-8')
        os.replace(temp, request)

        def read():
            try:
                value = json.loads(response.read_text(encoding='utf-8-sig'))
                return value if value['id'] == str(sequence) else None
            except (OSError, ValueError):
                return None
        result = wait(read, f'response {command}')
        if result['status'] != 'PASS':
            raise AssertionError(result)
        return result['value']

    def badge(label=None, visible=True):
        value = call(verb='input_indicator')
        return value if value['visible'] == visible and (label is None or value['label'] == label) else None

    def check(name, action):
        value = action()
        checks.append(dict(name=name, status='PASS', evidence=value))
        (root / 'checks.json').write_text(json.dumps(checks, ensure_ascii=False, indent=2), encoding='utf-8')
        print('PASS', name, flush=True)
        return value

    try:
        echo = start(executable, '--background')
        wait(lambda: (root / 'native-control').exists(), 'Echo test bridge')
        fixture = start(fixture_exe, str(root), 'Echo Input Indicator Fixture')
        wait(lambda: (root / 'native-ready.json').exists(), 'owned fixture')
        call(True, op='reset', control='single', text='Synthetic input', start=3, length=0)
        # An existing foreground app can deny a newly launched fixture's activation.
        # Grant foreground permission, then target only this owned fixture HWND.
        fixture_window = call(True, op='state')['window']
        user32.SetForegroundWindow.argtypes = [ctypes.c_void_p]
        user32.GetWindowThreadProcessId.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
        owner = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(fixture_window, ctypes.byref(owner))
        assert owner.value == fixture.pid, 'fixture HWND ownership changed'
        if user32.GetForegroundWindow() != fixture_window:
            user32.keybd_event(0x12, 0, 0, 0)
            user32.keybd_event(0x12, 0, 2, 0)
            user32.SetForegroundWindow(fixture_window)
        wait(lambda: user32.GetForegroundWindow() == fixture_window, 'owned fixture foreground')
        call(True, op='ime-english', control='single')
        first = check('english-state-and-default-enabled', lambda: wait(lambda: badge('EN'), 'EN indicator'))
        foreground = user32.GetForegroundWindow()
        assert foreground == first['sample']['window'], 'badge stole foreground focus'
        style = user32.GetWindowLongPtrW(first['hwnd'], -20)
        required = 0x08000000 | 0x80 | 0x20 | 0x80000
        assert style & required == required and style & 0x40000 == 0, hex(style)
        check('passive-tool-window', lambda: dict(ex_style=hex(style), foreground=foreground))
        # Observe native focus and size throughout real owned-fixture mode flips.
        from ctypes.wintypes import RECT
        user32.GetWindowRect.argtypes = [ctypes.c_void_p, ctypes.POINTER(RECT)]
        def badge_size():
            rect = RECT()
            assert user32.GetWindowRect(first['hwnd'], ctypes.byref(rect))
            return (rect.right - rect.left, rect.bottom - rect.top)
        fixed_size = badge_size()
        for operation, label in [('ime-chinese', '中'), ('ime-english', 'EN')]:
            call(True, op=operation, control='single')
            wait(lambda: badge(label), 'flip target mode')
            deadline = time.monotonic() + .25
            while time.monotonic() < deadline:
                assert user32.GetForegroundWindow() == foreground, 'flip took focus'
                assert badge_size() == fixed_size, 'flip resized native window'
                time.sleep(.01)
        check('mode-flip-preserves-native-size-and-input-focus', lambda: dict(size=fixed_size))
        # State-machine callbacks alone do not establish visible animation.
        # Exercise hide/refocus, then inspect actual software framebuffer spans.
        def trace_events():
            path = root / 'data/logs/input-indicator.jsonl'
            return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]

        # Reproduce the captured race without changing the actual focused target:
        # repeated accessibility focus notifications must revalidate in-place,
        # not clear a freshly displayed badge.
        churn_start = trace_events()[-1]['sequence']
        for _ in range(12):
            call(True, op='focus-signal', control='single')
            assert badge('EN'), 'same-target focus signal hid badge'
            time.sleep(.015)
        time.sleep(.3)
        churn_trace = [e for e in trace_events() if e['sequence'] > churn_start]
        starts = [e for e in churn_trace if e['event'] == 'retention-start']
        replacements = [e for e in churn_trace if e['event'] == 'retention-replaced']
        hides = [e for e in churn_trace if e['event'] == 'window-hide']
        assert starts and replacements, churn_trace
        assert not hides, churn_trace
        assert badge('EN'), 'same-target revalidation did not settle visible'
        check('same-target-focus-revalidation-does-not-flicker', lambda: dict(
            cycles=12, retention_starts=len(starts), retention_replacements=len(replacements)))

        # Diagnostics may expose only executable basenames, never document text,
        # titles, or full executable paths.
        diagnostic = [e for e in churn_trace if e['event'] == 'observation-changed']
        names = [e['details'].get('process_name') for e in diagnostic
                 if e['details'].get('process_name')]
        assert fixture_exe.name in names, names
        serialized = json.dumps(churn_trace, ensure_ascii=False)
        assert str(root).lower() not in serialized.lower(), 'diagnostics leaked a full path'
        forbidden = {'window_title', 'title', 'text', 'clipboard', 'document'}
        def keys(value):
            if isinstance(value, dict):
                for key, child in value.items():
                    yield key
                    yield from keys(child)
            elif isinstance(value, list):
                for child in value:
                    yield from keys(child)
        assert not (forbidden & set(keys(churn_trace))), 'diagnostics leaked content fields'
        check('indicator-log-is-content-free', lambda: dict(process_names=sorted(set(names))))

        flips = []
        for cycle in range(4):
            call(True, op='focus', control='readonly')
            wait(lambda: badge(visible=False), 'refocus hides badge')
            call(True, op='focus', control='single')
            call(True, op='ime-english', control='single')
            wait(lambda: badge('EN'), 'refocus restores EN')
            time.sleep(.2)
            first_sequence = trace_events()[-1]['sequence']
            call(True, op='ime-chinese', control='single')
            wait(lambda: badge('中'), 'refocus mode delivered')
            time.sleep(.2)
            trace = [e for e in trace_events() if e['sequence'] > first_sequence]
            delivered = next(e for e in trace if e['event'] == 'mode-delivered')
            if not delivered['details']['visible']:
                # Some IMEs restore focus during their mode command. A fresh
                # reveal deliberately settles directly instead of replaying old text.
                assert any(e['event'] == 'skip-hidden' for e in trace), trace
                assert badge('中'), 'fresh reveal has stale mode'
                flips.append(dict(cycle=cycle, first_reveal=True))
                continue
            started = next(e for e in trace if e['event'] == 'flip-start')
            frames = [e['details'] for e in trace if e['event'] == 'native-test-frame']
            assert any(0 < f['span'] < fixed_size[0] - 3 for f in frames), trace
            assert all((f['width'], f['height']) == fixed_size for f in frames), frames
            assert user32.GetForegroundWindow() == foreground, 'refocus flip stole focus'
            delay = started['utc_ms'] - delivered['utc_ms']
            assert delay < 75, ('passive redraw waited for sampling', delay)
            flips.append(dict(cycle=cycle, start_delay_ms=delay, spans=[f['span'] for f in frames]))
        assert sum('spans' in flip for flip in flips) >= 3, flips
        check('visible-flip-after-refocus', lambda: flips)
        call(True, op='ime-english', control='single')
        wait(lambda: badge('EN'), 'restore EN after refocus checks')
        time.sleep(.2)
        process = psutil.Process(echo.pid)
        cpu_start = sum(process.cpu_times()[:2])
        count_start = call(verb='input_indicator')['counts']
        quiet_start = time.monotonic()
        time.sleep(2)
        count_end = call(verb='input_indicator')['counts']
        elapsed = time.monotonic() - quiet_start
        samples = count_end[0] - count_start[0]
        probes = count_end[1] - count_start[1]
        assert samples <= elapsed * 15, (samples, elapsed)
        check('quiet-input-observation-budget', lambda: dict(seconds=elapsed, samples=samples,
            probes=probes, one_core_cpu_percent=100 * (sum(process.cpu_times()[:2]) - cpu_start) / elapsed))
        call(True, op='focus', control='single')
        wait(lambda: user32.GetForegroundWindow() == fixture_window, 'fixture foreground before themes')
        wait(lambda: badge('EN'), 'restore EN before themes')
        call(verb='input_indicator_capture', file='badge-light.png')
        for theme in ['dark', 'high-contrast', 'light']:
            call(verb='style_theme', file=theme)
            wait(lambda: badge('EN'), 'theme badge')
            time.sleep(.15)
            call(verb='input_indicator_capture', file=f'badge-{theme}-updated.png')
        check('theme-rendering', lambda: ['light', 'dark', 'high-contrast'])
        before = first['rect']
        call(True, op='selection', control='single', start=10, length=0)
        check('caret-follows-owned-input', lambda: wait(lambda: (v if (v := badge('EN')) and v['rect'] != before else None), 'caret movement'))
        for name in ['password', 'readonly']:
            call(True, op='focus', control=name)
            check(name + '-hidden', lambda: wait(lambda: badge(visible=False), name))
        call(True, op='focus', control='single')
        wait(lambda: badge('EN'), 'focus restored')
        call(verb='input_indicator_setting', paused=True, file='draft')
        check('draft-does-not-disable', lambda: wait(lambda: badge('EN'), 'unsaved indicator'))
        result = call(verb='input_indicator_setting', file='cancel')
        assert result['draft'] and result['saved'], result
        check('cancel-restores-draft', lambda: result)
        call(verb='input_indicator_setting', paused=True, file='save')
        check('saved-disable-hides', lambda: wait(lambda: badge(visible=False), 'disabled indicator'))
        disabled_counts = call(verb='input_indicator')['counts']
        time.sleep(.5)
        assert call(verb='input_indicator')['counts'] == disabled_counts
        check('disabled-stops-sampling', lambda: disabled_counts)
        call(verb='input_indicator_setting', paused=False, file='save')
        check('saved-enable-resumes', lambda: wait(lambda: badge('EN'), 'enabled indicator'))
        start_time = time.monotonic()
        call(True, op='ime-chinese', control='single')
        value = wait(lambda: badge('中'), 'Chinese IMM mode')
        check('owned-imm-chinese', lambda: dict(badge=value, elapsed_ms=round((time.monotonic() - start_time) * 1000)))
        time.sleep(.12)
        call(verb='input_indicator_capture', file='badge-chinese.png')
        call(True, op='ime-english', control='single')
        check('owned-imm-english', lambda: wait(lambda: badge('EN'), 'English IMM mode'))
        for name, clsid, profile in installed_profiles():
            active = call(True, op='ime-profile', control='single', clsid=clsid, profile=profile)
            wait(lambda: badge('中'), name + ' Chinese')
            times = []
            for chinese in [False, True, False, True]:
                begin = time.monotonic()
                call(True, op='ime-native-mode', control='single', chinese=chinese)
                wait(lambda: badge('中' if chinese else 'EN'), name + ' mode')
                times.append(round((time.monotonic() - begin) * 1000))
            check(name + '-owned-profile-mode-changes', lambda: dict(profile=active, elapsed_ms=times))
        call(True, op='ime-english', control='single')
        unchanged = call(True, op='state')['fields']['single']
        assert unchanged['text'] == 'Synthetic input' and unchanged['paste_attempts'] == 0
        check('observation-preserves-input', lambda: dict(text_unchanged=True, paste_attempts=0))
        call(True, op='allow', pid=echo.pid)
        subprocess.run([str(executable), '--settings'], env=env, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        wait(lambda: (v if (v := call(verb='metrics'))['visible'] and v['route'] == 'settings' else None), 'settings window')
        time.sleep(.2)
        call(verb='capture', file='settings-appearance.png')
        # Inspect only UIA roots belonging to our isolated Echo process.
        script = '''
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$targetProcessId = PROCESS_ID
$roots = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
    [System.Windows.Automation.TreeScope]::Children,
    [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,$targetProcessId))
foreach ($ownedRoot in $roots) {
    $general = $ownedRoot.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'通用'))
    if ($null -ne $general) {
        $invoke = $general.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $invoke.Invoke()
        Start-Sleep -Milliseconds 200
        break
    }
}
$foundCount = 0
foreach ($ownedRoot in $roots) {
    $foundCount += $ownedRoot.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'输入法提示')).Count
}
[Console]::WriteLine($foundCount)
'''.replace('PROCESS_ID', str(echo.pid))
        encoded = base64.b64encode(script.encode('utf-16-le')).decode('ascii')
        names = subprocess.check_output(['powershell.exe', '-NoProfile', '-EncodedCommand', encoded],
                                        creationflags=subprocess.CREATE_NO_WINDOW, timeout=15).decode().strip()
        assert int(names) > 0, 'Chinese settings must expose the translated indicator name'
        check('settings-indicator-chinese-accessible-name', lambda: '输入法提示')
        call(verb='capture', file='settings-general.png')
        check('settings-general-rendered', lambda: 'settings-general.png')
        checks.append(dict(name='physical-microsoft-doubao-codex-browser-mixed-dpi', status='NOT_RUN',
                           evidence='Synthetic fixture and IMM commands do not certify physical input or other applications.'))
    except Exception as error:
        checks.append(dict(name='acceptance', status='FAIL', error=str(error)))
        raise
    finally:
        (root / 'checks.json').write_text(json.dumps(checks, ensure_ascii=False, indent=2), encoding='utf-8')
        if len(children) > 1 and children[1].poll() is None:
            try:
                call(True, op='quit')
            except Exception:
                pass
        if children and children[0].poll() is None:
            subprocess.run([str(executable), '--quit'], env=env, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        for child in children:
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.terminate()
                child.wait(timeout=5)
        for log in logs:
            log.close()


if __name__ == '__main__':
    main()
