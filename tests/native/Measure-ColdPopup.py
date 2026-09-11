"""Process launch to complete settled popup pixels, using only an owned fixture.

This is distinct from first Alt+V after an already initialized manager. No input
is inserted. The observer matches the full changed region and its content edges.
"""
import argparse
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

import numpy as np
import psutil
from PIL import Image


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--count", type=int, default=30)
    args = parser.parse_args()
    root = args.root.resolve()
    if not 1 <= args.count <= 100:
        parser.error("count must be 1..100")
    if (root / "cold-results.json").exists():
        raise RuntimeError("Use a fresh evidence directory")
    marker = json.loads((root / "data/synthetic-fixture.json").read_text(encoding="utf-8-sig"))
    if marker.get("synthetic") is not True or marker.get("capture_enabled") is not False:
        raise RuntimeError("Requires an isolated capture-disabled fixture")
    exe = root / "echo-timing.exe"
    for proc in psutil.process_iter(["name"]):
        if (proc.info["name"] or "").lower() in ("echo-desktop.exe", "echo-timing.exe", "echo.exe"):
            raise RuntimeError("Another Echo process is running")
    env = dict(os.environ, ECHO_WINDOWS_ACCEPTANCE="1", ECHO_DATA_DIR=str(root / "data"))
    owned = []
    results = []
    fixture = None
    product = None
    title = "Echo Cold Timing Native"

    def register(pid, label):
        p = psutil.Process(pid)
        owned.append(dict(pid=pid, title=label, executable=p.exe(),
                          started_utc=datetime.datetime.fromtimestamp(p.create_time(), datetime.timezone.utc).isoformat()))
        (root / "owned-processes.json").write_text(json.dumps(owned), encoding="utf-8")

    def tool(op, pid, label, *values):
        p = subprocess.run([str(root / "EchoInlineDriver.exe"), op, str(root), str(pid), label, *map(str, values)],
                           env=env, capture_output=True, text=True, encoding="utf-8", timeout=25,
                           creationflags=subprocess.CREATE_NO_WINDOW)
        if p.returncode:
            raise RuntimeError(p.stdout + p.stderr)
        return json.loads(p.stdout.lstrip("\ufeff"))["value"]

    def stop_product():
        nonlocal product
        if product is not None and product.is_running():
            subprocess.run([str(exe), "--quit"], env=env, check=True, timeout=10,
                           creationflags=subprocess.CREATE_NO_WINDOW)
            product.wait(10)
        product = None

    try:
        fixture = subprocess.Popen([str(root / "EchoInlineFixture.exe"), str(root), title], env=env,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        register(fixture.pid, title)
        time.sleep(1)
        geometry = tool("geometry", fixture.pid, title)
        work=geometry["work"]
        scale=geometry["dpi"]/96
        tool("move", fixture.pid, title, work[0]+50, work[1]+50)
        tool("size-owned", fixture.pid, title, min(work[2]-work[0]-100, int(1200*scale)), min(work[3]-work[1]-100, int(900*scale)))
        geometry = tool("geometry", fixture.pid, title)
        (root / "cold-environment.json").write_text(json.dumps(dict(
            geometry=geometry, executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
            harness_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            driver_sha256=hashlib.sha256((root/"EchoInlineDriver.exe").read_bytes()).hexdigest(),
            fixture_executable_sha256=hashlib.sha256((root/"EchoInlineFixture.exe").read_bytes()).hexdigest(),
            endpoint="Process.Start(--echo-activate echo.quick_insert envelope) to settled full changed ROI and content edges",
            cold_definition="new process per sample; OS file/driver caches are not forcibly emptied",
            interval_censored=True, observer="GDI, factor 4; no UIA polling inside timed interval")), encoding="utf-8")
        for i in range(args.count):
            stop_product()
            tool("activate-owned", fixture.pid, title, "Inline fixture single")
            tool("english-owned", fixture.pid, title)
            # The enlarged owned fixture supplies a static background, including
            # behind the transparent side card; Codex updates cannot affect it.
            region=tool("geometry", fixture.pid, title)
            cx,cy,rx,by=region.get("caret") or tool("card", fixture.pid, title, "Inline fixture single")
            roi=[max(work[0],int(cx-50*scale)),max(work[1],int(cy-70*scale)),min(work[2],int(cx+980*scale)),min(work[3],int(by+620*scale))]
            (root / "capture-region.json").write_text(json.dumps(roi), encoding="utf-8")
            name = f"cold-{i:02d}"
            try:
                tool("measure-cold", fixture.pid, title, name, 3000)
            finally:
                identity = root / (name + ".process.json")
                if identity.exists():
                    value = json.loads(identity.read_text())
                    product = psutil.Process(value["pid"])
                    if Path(product.exe()).resolve() != exe:
                        raise RuntimeError("Cold product identity mismatch")
                    expected_created=datetime.datetime.fromisoformat(value["created_utc"].replace("Z", "+00:00")).timestamp()
                    if abs(product.create_time()-expected_created)>.001:
                        raise RuntimeError("Cold product PID creation time mismatch")
                    if value["session_id"]!=value["observer_session_id"]:
                        raise RuntimeError("Cold product started in another desktop session")
                    register(product.pid, "Echo Recall")
            if not tool("ready", product.pid, "Echo Recall"):
                raise RuntimeError("Cold popup missing semantic content")
            bounds=tool("geometry", product.pid, "Echo Recall")["window"]
            if bounds[0]<roi[0] or bounds[1]<roi[1] or bounds[2]>roi[2] or bounds[3]>roi[3]:
                raise RuntimeError("Capture omitted part of the cold popup")
            main_bounds=tool("card", product.pid, "Echo Recall", "History space")
            meta = json.loads((root / (name + ".frames.json")).read_text())
            frames = np.frombuffer(gzip.decompress((root / (name + ".bgra.gz")).read_bytes()), dtype=np.uint8)
            frames = frames.reshape(-1, meta["height"], meta["width"], 4)[:, :, :, :3]
            reference = np.median(frames[-5:], axis=0).astype(np.int16)
            changed = np.max(np.abs(reference - frames[0].astype(np.int16)), axis=-1) > 24
            edges = ((np.max(np.abs(reference - np.roll(reference, 1, axis=0)), axis=-1) > 20)
                     | (np.max(np.abs(reference - np.roll(reference, 1, axis=1)), axis=-1) > 20)) & changed
            if changed.sum() < 1000 or edges.sum() < 100:
                raise RuntimeError("Insufficient full-popup/content pixels")
            def rectangle(left,top,right,bottom):
                yy,xx=np.indices(changed.shape)
                origin=meta["origin"]; factor=meta["factor"]
                return (xx>=(left-origin[0])/factor)&(xx<=(right-origin[0])/factor)&(yy>=(top-origin[1])/factor)&(yy<=(bottom-origin[1])/factor)
            x,y,right,bottom=main_bounds
            header=rectangle(x+24*scale,y+12*scale,right-24*scale,y+64*scale)&changed
            body=rectangle(x+24*scale,y+78*scale,right-24*scale,bottom-28*scale)&edges
            side=~rectangle(x-4*scale,y-4*scale,right+4*scale,bottom+4*scale)&changed
            masks={"main_header":header,"main_content":body,"side_complete":side,"side_content":side&edges}
            if any(mask.sum()<40 for mask in masks.values()):
                raise RuntimeError("Insufficient main/side content pixels for full-frame evidence")
            matches = np.max(np.abs(frames.astype(np.int16) - reference), axis=-1) <= 20
            scores=np.minimum.reduce([matches[:,mask].mean(axis=1) for mask in masks.values()])
            ready = next((j for j in range(1, len(scores)-2) if min(scores[j:j+3]) >= .98), None)
            if ready is None:
                raise RuntimeError("No stable complete popup within capture")
            times = np.array(meta["frames"])
            result = dict(name=name, lower_ms=max(0, times[ready-1, 0]), upper_ms=times[ready, 1],
                          partitions={key:int(mask.sum()) for key,mask in masks.items()}, main_bounds=main_bounds,
                          changed_pixels=int(changed.sum()), content_pixels=int(edges.sum()),
                          capture_median_ms=float(np.median(times[1:, 1]-times[1:, 0])),
                          interval_p95_ms=float(np.percentile(np.diff(times[1:, 1]), 95)))
            Image.fromarray(reference.astype(np.uint8)[:, :, ::-1]).save(root / (name + ".settled.png"))
            results.append(result)
            (root / "cold-results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
            print(json.dumps(result), flush=True)
            stop_product()
    finally:
        stop_product()
        if fixture is not None and fixture.poll() is None:
            tool("close-owned", fixture.pid, title)
            fixture.wait(10)


if __name__ == "__main__":
    main()
