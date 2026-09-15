"""Style reload acceptance through the owned native-test bridge; no global input."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--executable', required=True)
    p.add_argument('--template', required=True)
    p.add_argument('--source', required=True)
    p.add_argument('--evidence', required=True)
    p.add_argument('--performance-only', action='store_true')
    args = p.parse_args()
    exe, template, source, root = map(lambda x: Path(x).resolve(),
                                     (args.executable, args.template, args.source, args.evidence))
    if os.environ.get('ECHO_WINDOWS_ACCEPTANCE') != '1':
        raise RuntimeError('Requires explicit isolated native acceptance authorization')
    marker = json.loads((template / 'synthetic-fixture.json').read_text(encoding='utf-8'))
    if marker.get('synthetic') is not True or marker.get('capture_enabled') is not False:
        raise RuntimeError('Only capture-disabled synthetic fixtures are allowed')
    root.mkdir(parents=True, exist_ok=False)
    data = root / 'data'
    assert data.resolve().parent == root
    shutil.copytree(template, data)
    config = root / 'styles.json'
    shutil.copyfile(source, config)
    doc = json.loads(config.read_text(encoding='utf-8'))
    env = dict(os.environ, ECHO_WINDOWS_ACCEPTANCE='1', ECHO_DATA_DIR=str(data),
               ECHO_NATIVE_TEST_ROOT=str(root), ECHO_MEMORY_TRACE_DIR=str(root),
               ECHO_DEV_STYLE_SOURCE=str(config))
    checks = []
    completed = False
    started = time.time_ns()
    out = (root / 'stdout.log').open('w', encoding='utf-8')
    err = (root / 'stderr.log').open('w', encoding='utf-8')
    child = subprocess.Popen([str(exe), '--history'], env=env, stdout=out, stderr=err,
                             creationflags=subprocess.CREATE_NO_WINDOW)

    def call(verb, **fields):
        control = root / 'native-control'
        control.mkdir(exist_ok=True)
        ident = uuid.uuid4().hex
        request = dict(id=ident, pid=child.pid, verb=verb, **fields)
        pending = control / 'request.pending'
        pending.write_text(json.dumps(request), encoding='utf-8')
        pending.replace(control / 'request.json')
        limit = time.monotonic() + 15
        while time.monotonic() < limit:
            if child.poll() is not None:
                raise RuntimeError(f'Owned process exited {child.returncode}')
            try:
                response = json.loads((control / 'response.json').read_text(encoding='utf-8'))
                if response['id'] == ident:
                    if response['status'] != 'PASS':
                        raise RuntimeError(response.get('error', response))
                    return response['value']
            except (FileNotFoundError, PermissionError, json.JSONDecodeError):
                pass
            time.sleep(.01)
        raise TimeoutError(verb)

    def wait(fn, timeout=10):
        limit = time.monotonic() + timeout
        while time.monotonic() < limit:
            result = fn()
            if result:
                return result
            time.sleep(.02)
        raise TimeoutError('condition')

    def ready():
        m = call('metrics')
        return m if m['ready'] and not m['loading'] and m['phase'] == 'Idle' else None

    def stable(s):
        return {k: s[k] for k in ('pid', 'query', 'selection', 'scroll', 'rows')}

    def save(value):
        pending = root / 'styles.pending'
        pending.write_text(json.dumps(value), encoding='utf-8')
        pending.replace(config)

    try:
        wait(lambda: (root / 'native-control').is_dir())
        call('ping')
        wait(ready)
        if not args.performance_only:
            call('style_theme', file='light')
            call('query', file='fixture')
            wait(ready)
            call('scroll', key=120)
            time.sleep(.05)
            before = call('style_state')
            assert before['rows'], 'Need matching synthetic content'
            assert before['scroll'] != 0, 'Need a nonzero scroll position for preservation coverage'
            call('capture', file='before.png')
            variants = [('color.row-text', '#777777', 'row_color', [119, 119, 119, 255]),
                        ('font.body', 17, 'font', 17),
                        ('font.weight-normal', 500, 'weight', 500),
                        ('space.1', 7, 'spacing', 7),
                        ('row.radius', 16, 'radius', 16),
                        ('color.search-text', '#c03652', 'search_text', [192, 54, 82, 255])]
            for token, value, field, expected in variants:
                doc['tokens'][token]['value'] = value
                begin = time.monotonic()
                save(doc)
                state = wait(lambda: (s if (s := call('style_state'))[field] == expected else None), 2)
                elapsed = (time.monotonic() - begin) * 1000
                assert elapsed < 1000, (token, elapsed)
                assert stable(state) == stable(before), (token, 'state changed')
                checks.append(dict(name=token, status='PASS', applied_ms=elapsed))
            assert state['rich'] != before['rich'], 'Existing match spans did not change color'
            call('capture', file='after.png')
            config.write_text('{', encoding='utf-8')
            time.sleep(.7)
            assert call('style_state') == state
            invalid = copy.deepcopy(doc)
            invalid['tokens']['font.body']['value'] = -1
            save(invalid)
            time.sleep(.7)
            assert call('style_state') == state
            save(doc)
            time.sleep(.7)
            assert call('style_state') == state
            doc['tokens']['font.body']['value'] = 18
            save(doc)
            recovered = wait(lambda: (s if (s := call('style_state'))['font'] == 18 else None), 2)
            assert stable(recovered) == stable(before)
            checks.append(dict(name='invalid-save-retains-state-and-recovers', status='PASS'))
            call('style_theme', file='dark')
            dark = call('style_state')
            assert dark['dark'] and stable(dark) == stable(before)
            call('capture', file='dark.png')
            call('style_theme', file='high-contrast')
            hc = call('style_state')
            assert hc['high_contrast'] and stable(hc) == stable(before)
            call('capture', file='high-contrast.png')
            checks.append(dict(name='dark-and-synthetic-high-contrast', status='PASS'))
            call('style_theme', file='light')
            call('query', file='')
            wait(ready)
            call('capture', file='side-cards.png')
            # Restore the source values before timing animations.
            save(json.loads(source.read_text(encoding='utf-8')))
            time.sleep(.7)
        for _ in range(6):
            call('step')
            wait(ready)
            time.sleep(.1)
        checks.append(dict(name='six-native-space-transitions', status='PASS'))
        completed = True
    finally:
        if child.poll() is None:
            subprocess.run([str(exe), '--quit'], env=env, capture_output=True, timeout=10,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            child.wait(timeout=15)
        out.close()
        err.close()
        events = []
        for path in root.glob('lifecycle-*.jsonl'):
            events.extend(json.loads(line) for line in path.read_text(encoding='utf-8').splitlines())
        frames = [e for e in events if e['state'] == 'frame_presented']
        durations = sorted(e['details']['render_present_us'] / 1000 for e in frames)
        result = dict(status='PASS' if completed else 'FAIL', executable=str(exe), sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
                      pid=child.pid, checks=checks, exit_code=child.returncode,
                      launch_to_first_frame_ms=(int(frames[0]['utc_ns']) - started) / 1e6 if frames else None,
                      frame_count=len(frames),
                      render_present_p95_ms=durations[min(len(durations)-1, int(len(durations)*.95))] if durations else None)
        (root / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps(result))


if __name__ == '__main__':
    main()
