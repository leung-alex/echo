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
    env = dict(os.environ, ECHO_DATA_DIR=str(root / 'data'), ECHO_NATIVE_TEST_ROOT=str(root), ECHO_RENDERER='software')
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
        first = check('english-state-and-default-enabled', lambda: wait(lambda: badge('EN'), 'EN indicator'))
        foreground = user32.GetForegroundWindow()
        assert foreground == first['sample']['window'], 'badge stole foreground focus'
        style = user32.GetWindowLongPtrW(first['hwnd'], -20)
        required = 0x08000000 | 0x80 | 0x20 | 0x80000
        assert style & required == required and style & 0x40000 == 0, hex(style)
        check('passive-tool-window', lambda: dict(ex_style=hex(style), foreground=foreground))
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
        check('settings-appearance-rendered', lambda: 'settings-appearance.png')
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
