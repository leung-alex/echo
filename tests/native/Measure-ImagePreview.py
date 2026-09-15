"""Isolated native image-preview benchmark; no clipboard capture or global input."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
import uuid
import psutil


def main():
    parser = argparse.ArgumentParser()
    for name in ('executable', 'template', 'evidence'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--scale', type=float, default=1.25)
    args = parser.parse_args()
    exe, template, root = (Path(v).resolve() for v in (args.executable, args.template, args.evidence))
    marker = json.loads((template / 'synthetic-fixture.json').read_text())
    assert marker['synthetic'] and not marker['capture_enabled']
    assert os.environ.get('ECHO_WINDOWS_ACCEPTANCE') == '1'
    root.mkdir(parents=True, exist_ok=False)
    shutil.copytree(template, root / 'data')
    env = dict(os.environ, ECHO_WINDOWS_ACCEPTANCE='1', ECHO_DATA_DIR=str(root / 'data'),
               ECHO_NATIVE_TEST_ROOT=str(root), ECHO_MEMORY_TRACE_DIR=str(root), SLINT_SCALE_FACTOR=str(args.scale))
    env.pop('ECHO_DEV_STYLE_SOURCE', None)
    stdout = (root / 'stdout.log').open('w')
    stderr = (root / 'stderr.log').open('w')
    start = time.time_ns()
    child = subprocess.Popen([str(exe), '--history'], env=env, stdout=stdout, stderr=stderr,
                             creationflags=subprocess.CREATE_NO_WINDOW)
    process = psutil.Process(child.pid)
    samples = []
    stop = threading.Event()

    def sample():
        while not stop.wait(.02):
            try:
                info = process.memory_info()
                samples.append((time.time_ns(), info.private, info.rss))
            except psutil.Error:
                return

    sampler = threading.Thread(target=sample)
    sampler.start()

    def call(verb, **fields):
        control = root / 'native-control'
        ident = uuid.uuid4().hex
        pending = control / 'pending.json'
        pending.write_text(json.dumps(dict(id=ident, pid=child.pid, verb=verb, **fields)))
        for attempt in range(100):
            try:
                pending.replace(control / 'request.json')
                break
            except PermissionError:
                if attempt == 99:
                    raise
                time.sleep(.01)
        until = time.monotonic() + 30
        while time.monotonic() < until:
            assert child.poll() is None, 'owned instance exited'
            try:
                response = json.loads((control / 'response.json').read_text())
                if response['id'] == ident:
                    assert response['status'] == 'PASS', response
                    return response['value']
            except (FileNotFoundError, PermissionError, json.JSONDecodeError):
                pass
            time.sleep(.01)
        raise TimeoutError(verb)

    passed = False
    first_image_ms = None
    try:
        until = time.monotonic() + 30
        while not (root / 'native-control').exists():
            assert time.monotonic() < until and child.poll() is None
            time.sleep(.01)
        last_bytes = -1
        stable_since = time.monotonic()
        while time.monotonic() < until:
            m = call('metrics')
            if m['thumbnails_bytes'] > 0 and first_image_ms is None:
                first_image_ms = (time.time_ns() - start) / 1e6
            if m['thumbnails_bytes'] != last_bytes:
                last_bytes = m['thumbnails_bytes']
                stable_since = time.monotonic()
            if m['ready'] and not m['loading'] and last_bytes > 0 and time.monotonic() - stable_since > .6:
                break
        else:
            raise TimeoutError('image readiness')
        settled_ms = (time.time_ns() - start) / 1e6
        call('capture', file='initial.png')
        initial = call('metrics')
        passes = []
        for name in ['cold-scroll', 'warm-scroll']:
            begin = time.time_ns()
            for direction in [False, True]:
                for _ in range(20):
                    call('scroll', key=160, shift=direction)
                    time.sleep(.04)
            end = time.time_ns()
            passes.append(dict(name=name, begin=begin, end=end, metrics=call('metrics')))
        call('capture', file='after-scroll.png')
        final = call('metrics')
        passed = True
    finally:
        stop.set()
        sampler.join()
        if child.poll() is None:
            subprocess.run([str(exe), '--quit'], env=env, timeout=15, creationflags=subprocess.CREATE_NO_WINDOW)
            child.wait(timeout=15)
        stdout.close()
        stderr.close()
    events = [json.loads(line) for p in root.glob('lifecycle-*.jsonl') for line in p.read_text().splitlines()]
    frames = [e for e in events if e['state'] == 'frame_presented']
    for phase in passes:
        values = sorted(e['details']['render_present_us'] / 1000 for e in frames if phase['begin'] <= int(e['utc_ns']) <= phase['end'])
        phase['frame_count'] = len(values)
        phase['render_present_p95_ms'] = values[min(len(values) - 1, int(len(values) * .95))] if values else None
    result = dict(status='PASS' if passed else 'FAIL', sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
                  scale=initial['scale_factor'], first_image_ms=first_image_ms, settled_ms=settled_ms,
                  peak_private_mib=max(s[1] for s in samples)/1048576,
                  final_private_mib=samples[-1][1]/1048576, initial=initial, final=final, phases=passes)
    (root / 'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({k: result[k] for k in ['status', 'first_image_ms', 'peak_private_mib', 'final_private_mib']}))


if __name__ == '__main__':
    main()
