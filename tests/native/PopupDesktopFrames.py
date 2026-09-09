"""Optional DXGI observer for owned popup fixtures; no input or application control."""
import concurrent.futures
import ctypes
import gzip
import importlib.metadata
import json
import time


def measure(root, name, send):
    import cv2
    import dxcam
    from dxcam.core.dxgi_duplicator import DXGIDuplicator
    from dxcam.core.stagesurf import StageSurface

    if importlib.metadata.version("dxcam") != "0.3.0":
        raise RuntimeError("This observer is pinned to dxcam 0.3.0")

    # dxcam 0.3.0 explicitly calls Release and then drops a comtypes-owned pointer,
    # which releases the same reference twice. In this standalone test process,
    # let comtypes release each owned reference once, including between trials.
    def release_stage(stage):
        stage.width = stage.height = 0
        stage.interface = None
        stage.texture = None

    def release_duplicator(duplicator):
        if duplicator.duplicator is not None:
            duplicator.release_frame()
            duplicator.duplicator = None

    StageSurface.release = release_stage
    DXGIDuplicator.release = release_duplicator

    region = tuple(json.loads((root / "capture-region.json").read_text()))
    factor = 4
    width = (region[2] - region[0]) // factor
    height = (region[3] - region[1]) // factor
    frames, ticks, presented = [], [], []

    def qpc():
        value = ctypes.c_longlong()
        ctypes.windll.kernel32.QueryPerformanceCounter(ctypes.byref(value))
        return value.value

    def sample(camera):
        start = qpc()
        frame = camera.grab(region=region, new_frame_only=False)
        end = qpc()
        if frame is None:
            return False
        frames.append(cv2.resize(frame, (width, height), interpolation=cv2.INTER_NEAREST).tobytes())
        ticks.append([start, end])
        presented.append(camera._duplicator.latest_frame_ticks)
        return True

    with dxcam.create(output_color="BGRA", backend="dxgi") as camera:
        for _ in range(100):
            if sample(camera):
                break
            time.sleep(.005)
        if not frames:
            raise RuntimeError("No desktop baseline frame")
        # The caller owns and validates SendInput. Both processes use Windows QPC.
        # No UIA or window readiness queries run on the capture thread.
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            pending = pool.submit(send)
            until = time.perf_counter() + 3
            while time.perf_counter() < until:
                sample(camera)
                time.sleep(.003)
            sent = pending.result()

    origin, frequency = sent["input_qpc"], sent["frequency"]
    times = [[(a - origin) * 1000 / frequency, (b - origin) * 1000 / frequency] for a, b in ticks]
    times[0] = [-1, -1]
    output = root / (name + ".bgra.gz")
    with gzip.open(output, "wb", compresslevel=1) as stream:
        for frame in frames:
            stream.write(frame)
    metadata = dict(
        name=name, width=width, height=height, origin=list(region[:2]), factor=factor,
        format="BGRA32", frames=times, frequency=frequency, input_qpc=origin,
        desktop_present_qpc=presented,
        source="DXGI desktop duplication; QPC brackets acquisition and staging copy; guarded SendInput",
        sampler="dxcam 0.3.0 with single-release COM ownership correction",
        note="Unchanged desktop frames can repeat; acquisition polling rate is not display refresh rate",
        file=str(output),
    )
    (root / (name + ".frames.json")).write_text(json.dumps(metadata), encoding="utf-8")
    return dict(file=str(output), frames=len(frames), duration_ms=times[-1][1], sampler="dxgi")
