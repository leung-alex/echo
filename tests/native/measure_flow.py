"""Isolated, opt-in synthetic benchmark; no desktop capture or OS key injection."""
from pathlib import Path
import argparse, hashlib, json, math, os, shutil, statistics, subprocess, time, uuid


def request(directory, pid, verb, **fields):
    control = directory / 'native-control'
    token = uuid.uuid4().hex
    value = dict(id=token, pid=pid, verb=verb, **fields)
    temporary = control / 'request.pending'
    temporary.write_text(json.dumps(value), encoding='utf-8')
    os.replace(temporary, control / 'request.json')
    until = time.monotonic() + 20
    while time.monotonic() < until:
        try:
            result = json.loads((control / 'response.json').read_text(encoding='utf-8'))
            if result['id'] == token:
                if result['status'] != 'PASS':
                    raise RuntimeError(result.get('error'))
                return result.get('value')
        except (FileNotFoundError, json.JSONDecodeError, PermissionError):
            pass
        time.sleep(0.005)
    raise TimeoutError(f'Owned test process did not reply to {verb}')


def distribution(values):
    ordered = sorted(values)
    return dict(samples=len(ordered), median_us=statistics.median(ordered),
                p95_us=ordered[min(len(ordered)-1, max(0, math.ceil(len(ordered)*.95)-1))], max_us=max(ordered)) if ordered else dict(samples=0)

def process_memory(process):
    """Win32 counters for the Popen-owned fixture process, not adapter totals."""
    import ctypes
    from ctypes import wintypes
    class Counters(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('page_faults', wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                'peak_working_set_bytes', 'working_set_bytes', 'peak_paged_pool_bytes',
                'paged_pool_bytes', 'peak_nonpaged_pool_bytes', 'nonpaged_pool_bytes',
                'pagefile_bytes', 'peak_pagefile_bytes', 'private_commit_bytes')]
    counter = Counters()
    counter.cb = ctypes.sizeof(counter)
    query = ctypes.WinDLL('psapi', use_last_error=True).GetProcessMemoryInfo
    query.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    query.restype = wintypes.BOOL
    if not query(int(process._handle), ctypes.byref(counter), counter.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    return dict(working_set_bytes=counter.working_set_bytes,
                private_commit_bytes=counter.private_commit_bytes,
                peak_working_set_bytes=counter.peak_working_set_bytes)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--exe', type=Path, required=True)
    p.add_argument('--template', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--cycles', type=int, default=20)
    p.add_argument('--scale', default='1')
    p.add_argument('--budget', default='')
    p.add_argument('--baseline', action='store_true', help='Record the old build without optimization gates')
    a = p.parse_args()
    if not 1 <= a.cycles <= 256:
        raise ValueError("cycles must be in 1..256")
    if os.environ.get('ECHO_WINDOWS_ACCEPTANCE') != '1':
        raise RuntimeError('Acceptance must be authorized')
    marker = json.loads((a.template/'synthetic-fixture.json').read_text())
    if not marker['synthetic'] or marker['capture_enabled']:
        raise RuntimeError('Refusing non-synthetic data')
    a.output = a.output.resolve(); a.exe = a.exe.resolve()
    a.output.mkdir(parents=True, exist_ok=False)
    shutil.copytree(a.template, a.output/'data')
    env = dict(os.environ, ECHO_DATA_DIR=str(a.output/'data'), ECHO_NATIVE_TEST_ROOT=str(a.output),
               ECHO_RENDERER='femtovg-wgpu', SLINT_SCALE_FACTOR=a.scale)
    if a.budget: env['ECHO_ACCEPTANCE_GRAPHICS_BUDGET'] = a.budget
    log = (a.output/'application.log').open('w', encoding='utf-8')
    process = subprocess.Popen([str(a.exe), '--history'], env=env, stdout=log, stderr=log)
    report = dict(status='FAIL', synthetic=True, exe=str(a.exe), pid=process.pid,
                  sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(), scale=a.scale, budget=a.budget)
    try:
        deadline = time.monotonic()+30
        while not (a.output/'native-control').exists():
            if process.poll() is not None: raise RuntimeError('Application exited at startup')
            if time.monotonic() > deadline: raise TimeoutError('Native test bridge did not start')
            time.sleep(.05)
        while not request(a.output, process.pid, 'metrics')['ready']:
            if time.monotonic() > deadline: raise TimeoutError('Initial rows did not arrive')
            time.sleep(.05)
        time.sleep(1)
        report['process_memory_before'] = process_memory(process)
        request(a.output, process.pid, 'reset_metrics')
        for i in range(a.cycles):
            request(a.output, process.pid, 'step')
            time.sleep(.38)
        metrics = request(a.output, process.pid, 'metrics')
        report['raw'] = metrics
        report['navigation'] = distribution(metrics['navigation_us'])
        for name in ('cpu_frame_us', 'frame_interval_us', 'input_to_first_render_us', 'capture_us'):
            report[name] = distribution(metrics['graphics'][name])
        report['readbacks'] = metrics['graphics']['readbacks']
        # Do not mistake the final debounced cache preparation for an idle animation loop.
        time.sleep(1.5)
        before = request(a.output, process.pid, 'metrics')['graphics']['stats']
        time.sleep(2)
        after = request(a.output, process.pid, 'metrics')['graphics']['stats']
        report['idle_compositor_draws'] = after[2]-before[2]
        report['process_memory_after'] = process_memory(process)
        request(a.output, process.pid, 'capture', file='rest.png')
        failures = []
        if not metrics['ready'] or metrics.get('phase') != 'Idle': failures.append('content not interactive')
        if not a.baseline:
            if report['readbacks'] != 0: failures.append('GPU readbacks on the production path')
            if report['idle_compositor_draws'] != 0: failures.append('compositor kept drawing while idle')
            if metrics['graphics']['stats'][1] > 4: failures.append('panel count exceeded four')
            if metrics['graphics']['stats'][0] > metrics['graphics'].get('motion_texture_limit', 64*1024*1024): failures.append('texture budget exceeded')
        report['failures'] = failures
        report['status'] = 'FAIL' if failures else 'PASS'
    except Exception as exc:
        report['error'] = str(exc)
    finally:
        try:
            shutdown = subprocess.run([str(a.exe), '--quit'], env=env, timeout=15, check=False)
            code = process.wait(timeout=15)
            report['exit_code'] = code
            if code != 0 or shutdown.returncode != 0:
                report['status'] = 'FAIL'
                report['shutdown_error'] = f'resident={code}, handoff={shutdown.returncode}'
        except Exception as exc:
            report['status'] = 'FAIL'
            report['shutdown_error'] = str(exc)
            if process.poll() is None:
                process.kill()  # Only the Popen-owned, isolated synthetic test instance.
                process.wait(timeout=5)
        finally:
            log.close()
        (a.output/'metrics.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        print(json.dumps({k:v for k,v in report.items() if k != 'raw'}, indent=2), flush=True)
    if report['status'] != 'PASS': raise SystemExit(1)

if __name__ == '__main__':
    main()
