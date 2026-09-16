"""Real Windows SpaceChoice regression; only capture-disabled synthetic data.

Run with an echo-desktop --features native-test executable. The first check
reproduces the popup row's destroyed-parent panic on the unfixed application.
"""
import argparse
import datetime
import hashlib
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
    parser.add_argument('--template', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--repro-only', action='store_true')
    args = parser.parse_args()
    if os.environ.get('ECHO_WINDOWS_ACCEPTANCE') != '1':
        raise RuntimeError('Explicit native acceptance authorization required')
    repo = Path(__file__).resolve().parents[2]
    evidence = args.evidence.resolve()
    marker = json.loads((args.template / 'synthetic-fixture.json').read_text(encoding='utf-8-sig'))
    if marker.get('synthetic') is not True or marker.get('capture_enabled') is not False:
        raise RuntimeError('Only capture-disabled synthetic fixtures are allowed')
    evidence.mkdir(parents=True, exist_ok=False)
    shutil.copytree(args.template, evidence / 'data')
    exe = evidence / 'echo-acceptance.exe'
    shutil.copy2(args.executable, exe)
    pdb = args.executable.with_suffix('.pdb')
    if pdb.exists():
        shutil.copy2(pdb, exe.with_suffix('.pdb'))
    framework = Path(os.environ['WINDIR']) / 'Microsoft.NET/Framework64/v4.0.30319'
    references = [str(framework / 'WPF' / name) for name in
                  ['UIAutomationClient.dll', 'UIAutomationTypes.dll', 'WindowsBase.dll']]
    references += ['System.Drawing.dll', 'System.Windows.Forms.dll', 'System.Web.Extensions.dll',
                   'System.IO.Compression.dll', 'System.IO.Compression.FileSystem.dll']
    for name, sources in [('EchoDriver', ['EchoDriver', 'EchoBenchmarks', 'EchoComposition']),
                          ('EchoInlineDriver', ['EchoInlineDriver'])]:
        subprocess.run([str(framework / 'csc.exe'), '/nologo', '/target:exe',
                        '/out:' + str(evidence / (name + '.exe')),
                        *['/reference:' + r for r in references],
                        *[str(repo / 'tests/native' / (s + '.cs')) for s in ['EchoUi', *sources]]],
                       check=True, creationflags=subprocess.CREATE_NO_WINDOW)
    env = dict(os.environ, ECHO_DATA_DIR=str(evidence / 'data'),
               ECHO_NATIVE_TEST_ROOT=str(evidence), ECHO_ACCEPTANCE_RUN_ROOT=str(evidence),
               ECHO_RENDERER='software', RUST_BACKTRACE='full')
    process = None
    logs = []
    checks = []
    calls = []
    run = 0
    success = False

    def start(*argv):
        nonlocal process, run
        run += 1
        stdout = open(evidence / f'{run}-stdout.log', 'w', encoding='utf-8')
        stderr = open(evidence / f'{run}-stderr.log', 'w', encoding='utf-8')
        logs.extend([stdout, stderr])
        process = subprocess.Popen([str(exe), *argv], env=env, stdout=stdout, stderr=stderr,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        actual = psutil.Process(process.pid)
        (evidence / 'owned-processes.json').write_text(json.dumps([dict(
            pid=process.pid, title='Echo Recall', executable=actual.exe(),
            started_utc=datetime.datetime.fromtimestamp(actual.create_time(), datetime.timezone.utc).isoformat()
        )]), encoding='utf-8')
        wait(lambda: ui('dump'), 'window appears')

    def tool(driver, op, *argv):
        if process.poll() is not None:
            raise RuntimeError(f'Echo exited: code={process.returncode}')
        command = [str(evidence / (driver + '.exe')), op]
        if driver == 'EchoInlineDriver':
            command.append(str(evidence))
        command += [str(process.pid), 'Echo Recall', *map(str, argv)]
        result = subprocess.run(command, env=env, capture_output=True, encoding='utf-8',
                                timeout=20, creationflags=subprocess.CREATE_NO_WINDOW)
        calls.append(dict(operation=op, args=argv, code=result.returncode,
                          stdout=result.stdout, stderr=result.stderr))
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        return json.loads(result.stdout.lstrip('\ufeff'))['value']

    def ui(op, *argv):
        return tool('EchoDriver', op, *argv)

    def pointer(op, *argv):
        return tool('EchoInlineDriver', op, *argv)

    def wait(fn, label):
        until = time.monotonic() + 10
        error = None
        while time.monotonic() < until:
            if process.poll() is not None:
                raise RuntimeError(f'{label}: Echo exited: code={process.returncode}')
            try:
                value = fn()
                if value:
                    return value
            except Exception as exc:
                error = exc
            time.sleep(.05)
        raise RuntimeError(f'{label}: {error}')

    def activate(*argv):
        subprocess.run([str(exe), *argv], env=env, check=True, timeout=10,
                       creationflags=subprocess.CREATE_NO_WINDOW)

    def stop():
        if process and process.poll() is None:
            activate('--quit')
            try:
                process.wait(3)
            except subprocess.TimeoutExpired:
                # A failing scenario can leave the unsaved-changes dialog open.
                # This handle belongs only to the child launched above.
                process.kill()
                process.wait(5)
                calls.append(dict(operation='cleanup-owned-process', code=process.returncode))

    try:
        start('--settings')
        chinese = '设置' in ui('dump')
        section, combo, save, cancel = ('空间', '主屏幕', '保存更改', '取消') if chinese else (
            'Spaces', 'Home screen', 'Save', 'Cancel')
        ui('invoke', section)
        pointer('activate-owned', section)
        state = ui('metrics')
        spaces = state['spaces']
        if len(spaces) != 4 or spaces[0]['id'] != 1 or spaces[1]['id'] != 2:
            raise RuntimeError('This regression requires History, Favorites and two synthetic custom spaces')
        def label(index):
            return ('上次使用的空间' if chinese else 'Last used space') if index == len(spaces) else (
                ('历史记录' if index == 0 else '收藏') if chinese and index < 2 else spaces[index]['title'])

        def choose(index):
            expected = label(index)
            ui('expand', combo)
            rows = wait(lambda: ui('popup-items'), 'popup accessibility rows')
            row = next(r for r in rows if r['name'] == expected)
            x, y, width, height = row['bounds']
            if width <= 0 or height <= 0:
                raise RuntimeError('Popup row has no clickable bounds')
            ui('capture', str(evidence / f'popup-{len(checks)}.png'))
            pointer('click-point', int(x + width / 2), int(y + height / 2))
            wait(lambda: ui('read', combo) == expected, f'selection {expected}')
            assert ui('metrics')['settings']['valid']
            checks.append(dict(name=f'mouse-select-{index}', status='PASS'))

        choose(1)
        if not args.repro_only:
            for index in [2, len(spaces), 0, 1, 2, 0]:
                choose(index)
            for index in [1, 2, len(spaces), 0]:
                ui('expand', combo)
                ui('popup-select', label(index))
                wait(lambda: ui('read', combo) == label(index), 'accessible popup selection')
            checks.append(dict(name='accessible-default-selection-all-options', status='PASS'))
            # Keyboard commits and dismissal use the same popup owner as clicks.
            before = ui('read', combo)
            for dismiss_key in [27, 9]:
                ui('expand', combo)
                ui('key', 40)
                ui('key', dismiss_key)
                assert ui('read', combo) == before
            ui('expand', combo)
            ui('key', 40)
            ui('key', 13)
            wait(lambda: ui('read', combo) == ('收藏' if chinese else 'Favorites'), 'keyboard commit')
            checks.append(dict(name='keyboard-commit-escape-and-tab-cancel', status='PASS'))

            def settings():
                activate('--settings')
                wait(lambda: ui('metrics')['route'] == 'settings', 'settings route')
                ui('invoke', section)
                pointer('activate-owned', section)

            def ready(target):
                def matches():
                    current = ui('metrics')
                    return (current['route'] == 'history' and current['space'] == target
                            and current['phase'] == 'Idle' and current['ready']
                            and str(current['interaction']) == str(target))
                wait(matches, f'active space {target}')

            for index in [1, 2, len(spaces), 0]:
                if index != 1:
                    settings()
                choose(index)
                previous = ui('metrics')['space']
                key = 'last' if index == len(spaces) else str(spaces[index]['id'])
                ui('invoke', save)
                wait(lambda: not ui('metrics')['settings']['dirty'], 'settings saved')
                assert ui('metrics')['settings']['ui']['startup_space'] == key
                target = previous if key == 'last' else int(key)
                ui('close')
                wait(lambda: not ui('metrics')['visible'], 'hide before reopening')
                activate()
                ready(target)
                stop()
                start()
                ready(target)
                assert ui('metrics')['settings']['ui']['startup_space'] == key
                checks.append(dict(name=f'save-reopen-restart-{key}', status='PASS'))

            settings()
            persisted = ui('metrics')['settings']['ui']['startup_space']
            choose(2)
            ui('invoke', cancel)
            # Cancel routes through the existing unsaved-changes confirmation.
            discard = '放弃更改' if chinese else 'Discard changes'
            wait(lambda: discard in ui('dump') or ui('metrics')['route'] == 'history', 'cancel decision')
            if discard in ui('dump'):
                ui('invoke', discard)
            wait(lambda: ui('metrics')['route'] == 'history', 'cancel settings')
            assert ui('metrics')['settings']['ui']['startup_space'] == persisted
            settings()
            assert ui('read', combo) == ('历史记录' if chinese else 'History')
            checks.append(dict(name='cancel-preserves-saved-choice', status='PASS'))

            # Use the empty synthetic space so deletion never moves real content.
            empty = next(s for s in spaces[2:] if s['count'] == 0)
            choose(spaces.index(empty))
            ui('invoke', save)
            wait(lambda: not ui('metrics')['settings']['dirty'], 'save empty space choice')
            group = ('空间：' if chinese else 'Space ') + empty['title']
            ui('group-invoke', group, '编辑空间' if chinese else 'Edit space')
            ui('value', '空间名称' if chinese else 'Space name', 'Startup renamed')
            ui('invoke', '保存' if chinese else 'Save')
            wait(lambda: ui('read', combo) == 'Startup renamed', 'renamed choice label')
            group = ('空间：' if chinese else 'Space ') + 'Startup renamed'
            ui('group-invoke', group, '上移空间' if chinese else 'Move space up')
            wait(lambda: ui('metrics')['spaces'][2]['id'] == empty['id'], 'reordered space')
            assert ui('read', combo) == 'Startup renamed'
            activate()
            ready(empty['id'])
            stop()
            start()
            ready(empty['id'])
            checks.append(dict(name='rename-reorder-retains-identity-after-restart', status='PASS'))
            settings()
            ui('group-invoke', group, '删除空间' if chinese else 'Delete space')
            # The confirmation starts on Cancel; Tab moves to the accept button.
            ui('key', 9)
            ui('key', 13)
            wait(lambda: all(s['id'] != empty['id'] for s in ui('metrics')['spaces']), 'space deletion')
            activate()
            ready(1)
            stop()
            start()
            ready(1)
            checks.append(dict(name='deleted-startup-space-falls-back-after-restart', status='PASS'))
        (evidence / 'checks.json').write_text(json.dumps(checks, indent=2), encoding='utf-8')
        success = True
    finally:
        exit_code = process.poll() if process else None
        stop()
        for log in logs:
            log.close()
        (evidence / 'result.json').write_text(json.dumps(dict(
            status='PASS' if success else 'FAIL', checks=checks, unexpected_exit_code=exit_code,
            executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), calls=calls), ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(dict(status='PASS', checks=len(checks), evidence=str(evidence))))


if __name__ == '__main__':
    main()
