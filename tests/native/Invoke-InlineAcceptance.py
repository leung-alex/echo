"""Authorized Windows inline-completion acceptance with synthetic owned inputs.
Every input driver validates PID, image path, process creation time, and foreground.
This is not evidence for physical IME or applications that were not actually tested.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone

PAYLOAD = "echo-perf-text-0013 — Reusable content, available when you need it."
QUERY = "echo-perf-text-0013"
TITLE = "Echo Recall"

def atomic(path: Path, value: object) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")
    os.replace(temporary, path)

def source_head(root: Path) -> str:
    """Read local identity without allowing a metadata subprocess to stall tests."""
    try:
        head = (root / ".git" / "HEAD").read_text(encoding="ascii").strip()
        if not head.startswith("ref: "):
            return head
        ref = head[5:]
        loose = root / ".git" / ref
        if loose.exists():
            return loose.read_text(encoding="ascii").strip()
        for line in (root / ".git" / "packed-refs").read_text(encoding="ascii").splitlines():
            if line.endswith(" " + ref):
                return line.split()[0]
    except (OSError, UnicodeError):
        pass
    return "unavailable; executable SHA256 is recorded"

class Run:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.root = args.root.resolve()
        self.evidence = args.evidence.resolve()
        if self.evidence.exists():
            raise RuntimeError("Evidence directory must be new")
        self.evidence.mkdir(parents=True)
        marker = json.loads((args.template / "synthetic-fixture.json").read_text(encoding="utf-8-sig"))
        if not marker.get("synthetic") or marker.get("capture_enabled"):
            raise RuntimeError("Only capture-disabled synthetic data may be tested")
        shutil.copytree(args.template, self.evidence / "data")
        self.env = dict(os.environ, ECHO_WINDOWS_ACCEPTANCE="1", ECHO_DATA_DIR=str(self.evidence / "data"),
                        ECHO_ACCEPTANCE_RUN_ROOT=str(self.evidence), ECHO_RENDERER=args.renderer,
                        ECHO_DISABLE_GLOBAL_HOTKEY="0")
        if args.native_test:
            self.env["ECHO_NATIVE_TEST_ROOT"] = str(self.evidence)
        else:
            self.env.pop("ECHO_NATIVE_TEST_ROOT", None)
        self.checks: list[dict] = []
        self.records: list[dict] = []
        self.processes: list[subprocess.Popen] = []
        self.echo: subprocess.Popen | None = None
        self.native: subprocess.Popen | None = None
        self.native_title = "Echo Inline Native Fixture " + uuid.uuid4().hex
        self.browser: subprocess.Popen | None = None
        self.browser_title = "Echo Inline Composer Fixture"
        self.logs: list = []
        self.started = datetime.now(timezone.utc).isoformat()
        atomic(self.evidence / "identity.json", {
            "schema": "echo.inline.acceptance.v1", "started": self.started,
            "executable": str(args.executable.resolve()),
            "sha256": hashlib.sha256(args.executable.read_bytes()).hexdigest(),
            "renderer": args.renderer, "native_test": args.native_test,
            "machine": os.environ.get("COMPUTERNAME"),
            "head": source_head(self.root),
        })

    def launch(self, exe: Path, arguments: list[str], label: str) -> subprocess.Popen:
        print("LAUNCH", label, datetime.now(timezone.utc).isoformat(), flush=True)
        log = (self.evidence / (label + ".log")).open("wb")
        self.logs.append(log)
        process = subprocess.Popen([str(exe), *arguments], env=self.env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        self.processes.append(process)
        return process

    def record(self, process: subprocess.Popen, title: str) -> None:
        # Query the actual process image and birth time, not an assumed command path.
        command = (
            f"$p=Get-Process -Id {process.pid}; "
            "[ordered]@{pid=$p.Id;executable=$p.MainModule.FileName;started_utc=$p.StartTime.ToUniversalTime().ToString('o');started_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc()}|ConvertTo-Json -Compress"
        )
        def probe():
            result = subprocess.run(["pwsh", "-NoProfile", "-Command", command], capture_output=True,
                                    text=True, encoding="utf-8", errors="replace", timeout=8)
            if result.returncode != 0:
                return None
            value = json.loads(result.stdout.lstrip("\ufeff"))
            return value if value.get("executable") else None
        value = self.wait(probe, "actual process image", 10)
        value["title"] = title
        self.records.append(value)
        atomic(self.evidence / "owned-processes.json", self.records)

    def wait(self, probe, description: str, timeout: float = 8):
        end = time.monotonic() + timeout
        last = None
        while time.monotonic() < end:
            try:
                last = probe()
                if last:
                    return last
            except (RuntimeError, FileNotFoundError, json.JSONDecodeError, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(0.03)
        raise RuntimeError(f"Timed out: {description}; last={last}")

    def tool(self, exe: str, arguments: list[str], timeout: float = 15):
        result = subprocess.run([str(self.args.tools / exe), *arguments], env=self.env,
                                capture_output=True, text=True, encoding="utf-8", errors="strict", timeout=timeout)
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        value = json.loads(result.stdout.lstrip("\ufeff"))
        if value.get("status") != "PASS":
            raise RuntimeError(str(value))
        return value.get("value")

    def d(self, operation: str, *args):
        assert self.echo is not None
        return self.tool("EchoDriver.exe", [operation, str(self.echo.pid), TITLE, *map(str, args)])

    def f(self, operation: str, process: subprocess.Popen, title: str, *args):
        return self.tool("EchoInlineDriver.exe", [operation, str(self.evidence), str(process.pid), title, *map(str, args)])

    def n(self, operation: str, **arguments):
        request = uuid.uuid4().hex
        atomic(self.evidence / "native-command.json", dict(id=request, op=operation, **arguments))
        def reply():
            value = json.loads((self.evidence / "native-response.json").read_text(encoding="utf-8-sig"))
            if value.get("id") != request:
                return None
            if value.get("status") != "PASS":
                raise RuntimeError(str(value))
            return value["value"]
        return self.wait(reply, "native fixture command " + operation, 6)

    def reset(self, control="single", text="pre| |post", start=4, length=0):
        if self.echo and self.echo.poll() is None and self.d("exists"):
            self.d("close")
            self.wait(lambda: not self.d("exists"), "Echo hidden")
        state = self.n("reset", control=control, text=text, start=start, length=length)
        if not state["foreground"] or not state["fields"][control]["focused"]:
            # A real click on a validated, unobscured owned control gives focus.
            # No fake-Alt or cross-process AttachThreadInput workaround is used.
            self.f("activate-owned", self.native, self.native_title, "Inline fixture " + control)
            state = self.n("reset", control=control, text=text, start=start, length=length)
            if not state["foreground"] or not state["fields"][control]["focused"]:
                raise RuntimeError("Owned fixture did not obtain input focus")
        return state

    def state(self, control="single"):
        return self.n("state")["fields"][control]

    def metrics(self):
        return self.d("metrics")

    def inline_ready(self, query: str | None = None):
        if not self.d("exists"):
            return False
        if self.args.native_test:
            value = self.metrics()
            if value["inline"]["active"] and value["inline"].get("provider") and value["ready"] and not value["loading"]:
                return value if query is None or value["query"] == query else False
            if not value["inline"]["pending"] and not value["inline"]["active"]:
                raise RuntimeError("Expected inline mode, observed compatibility: " + value["inline"]["status"])
            return False
        dump = self.d("dump")
        ready = "History space" in dump and "ControlType.Edit | Search clipboard history" not in dump
        if query == QUERY:
            ready = ready and ("1 matches" in dump or "1 match" in dump) and QUERY in dump
        elif query and (query.startswith("not-found") or query.startswith("no-result") or query.startswith("邮箱")):
            ready = ready and "No matches in this space" in dump
        return ready

    def open_inline(self, query=""):
        self.f("hotkey", self.native, self.native_title, "Alt+V")
        value = self.wait(lambda: self.inline_ready(query), "inline popup ready", 12)
        state = self.n("state")
        if not state["foreground"] or state["deactivations"] != 0:
            raise RuntimeError("Echo took activation away from the original input")
        dump = self.d("dump")
        if "ControlType.Edit | Search clipboard history" in dump:
            raise RuntimeError("Inline mode still exposes its own search field")
        return value

    def type_query(self, query=QUERY):
        self.f("text", self.native, self.native_title, query)
        return self.wait(lambda: self.inline_ready(query), "actual composer query propagated", 10)

    def confirm_ready(self):
        if self.args.native_test:
            return self.wait(lambda: self.metrics()["inline"].get("readiness", [0] * 8)[7] == 1,
                             "current query result is armed for confirmation", 5)
        return True

    def enter(self, control="single", held=False, expected=None):
        self.confirm_ready()
        self.f("held-enter" if held else "key", self.native, self.native_title, *([] if held else [13]))
        self.wait(lambda: not self.d("exists"), "replacement acknowledged and popup hidden", 8)
        state = self.state(control)
        if state["enter_count"] != 0:
            raise RuntimeError("Enter reached the native input's submit handler")
        if expected is not None and state["text"] != expected:
            raise RuntimeError(f"Wrong replacement: actual={state['text']!r}, expected={expected!r}")
        return state

    def check(self, name: str, function):
        if self.args.only and name not in self.args.only.split(","):
            return
        print("CHECK", name, datetime.now(timezone.utc).isoformat(), flush=True)
        try:
            value = function()
            self.checks.append({"name": name, "status": "PASS", "actual": value})
            print("PASS", name, flush=True)
            atomic(self.evidence / "checks.json", self.checks)
        except Exception as error:
            self.checks.append({"name": name, "status": "FAIL", "error": str(error)})
            atomic(self.evidence / "checks.json", self.checks)
            try:
                if self.echo and self.echo.poll() is None and self.args.native_test:
                    atomic(self.evidence / ("failure-" + name + ".last-metrics.json"), self.metrics())
                if self.echo and self.echo.poll() is None and self.d("exists"):
                    (self.evidence / ("failure-" + name + ".uia.txt")).write_text(self.d("dump"), encoding="utf-8")
                    if self.args.native_test:
                        atomic(self.evidence / ("failure-" + name + ".metrics.json"), self.metrics())
            except Exception:
                pass
            if self.browser and self.browser.poll() is None:
                try: atomic(self.evidence / ("failure-" + name + ".browser.json"), self.browser_state())
                except Exception: pass
            raise

    def shot(self, name: str):
        if self.args.native_test:
            self.d("capture", str(self.evidence / (name + ".png")))

    def start_processes(self):
        if not self.tool("EchoInlineDriver.exe", ["probe",str(self.evidence),"Alt+V"])["available"]:
            raise RuntimeError("Alt+V is owned by another instance; no foreground test was started")
        self.echo = self.launch(self.args.executable, ["--background"], "echo")
        self.env["ECHO_ACCEPTANCE_PID"] = str(self.echo.pid)
        self.record(self.echo, TITLE)
        self.native = self.launch(self.args.tools / "EchoInlineFixture.exe", [str(self.evidence), self.native_title], "native-fixture")
        self.record(self.native, self.native_title)
        self.wait(lambda: (self.evidence / "native-ready.json").exists(), "native fixture ready")
        self.wait(lambda: not self.tool("EchoInlineDriver.exe", ["probe", str(self.evidence), "Alt+V"])["available"], "Alt+V registered")

    def native_checks(self):
        self.start_processes()
        self.check("installed-chinese-ime-confirmation-and-insertion", self.test_installed_ime)
        self.check("startup-independent-search-does-not-flash", self.test_independent_streaming)
        self.check("f6-independent-search-does-not-flash", lambda:self.test_independent_streaming(True))
        self.check("fuzzy-words-and-trailing-spaces-keep-actions-stable", self.test_fuzzy_words)
        self.check("first-popup-never-activates", self.test_first)
        self.check("composer-query-and-adaptive-height", self.test_height)
        self.check("enter-replaces-query-not-prefix-or-suffix", lambda: self.enter(expected="pre|" + PAYLOAD + " |post"))
        self.check("real-top-and-bottom-input-placement", self.test_input_placement)
        self.check("held-enter-never-leaks-submit-after-close", self.test_held)
        self.check("no-results-enter-is-consumed", self.test_empty)
        self.check("fast-new-query-cannot-use-old-result", self.test_fast)
        self.check("query-backspace-and-unicode-are-observed", self.test_unicode)
        self.check("preselected-query-replaced-exactly", self.test_selection)
        self.check("paste-into-query-is-observed", self.test_pasted_query)
        self.check("arrows-move-suggestion-not-original-caret", self.test_arrows)
        self.check("outside-range-movement-cancels", self.test_outside)
        self.check("multiline-native-input-replacement", lambda: self.test_control("multiline"))
        self.check("rich-edit-native-input-replacement", lambda: self.test_control("rich"))
        self.check("unsupported-protected-input-is-explicit-compatibility", self.test_password)
        self.check("f6-preserves-query-and-opens-independent-search", self.test_f6)
        self.check("streaming-filter-never-clears-the-panel", self.test_streaming_filter)

    def control_request(self, verb, **fields):
        ident = uuid.uuid4().hex
        atomic(self.evidence / "native-control" / "request.json", dict(id=ident,pid=self.echo.pid,verb=verb,**fields))
        def response():
            value=json.loads((self.evidence / "native-control" / "response.json").read_text(encoding="utf-8"))
            if value.get("id") != ident: return None
            if value.get("status") != "PASS": raise RuntimeError(str(value))
            return value["value"]
        return self.wait(response,"native trace " + verb,12)

    def test_installed_ime(self):
        self.reset();self.open_inline()
        self.n("ime-chinese",control="single")
        self.f("key",self.native,self.native_title,ord("N"))
        self.f("key",self.native,self.native_title,ord("I"))
        composed=self.wait(lambda:s if (s:=self.state())["ime"]["composition_bytes"]>0 else None,"installed Chinese IME started",8)
        atomic(self.evidence/"ime-window-metadata.json",self.f("ime-metadata",self.native,self.native_title))
        self.wait(lambda:self.metrics()["inline"]["composing"],"Echo observes real IME composition",8)
        self.shot("installed-ime-composing")
        self.f("key",self.native,self.native_title,13)
        committed=self.wait(lambda:s if (s:=self.state())["ime"]["composition_bytes"]==0 else None,"Enter confirms IME",8)
        if committed["enter_count"]!=0 or not self.d("exists"):
            raise RuntimeError("IME confirmation submitted the input or closed the completion session")
        self.n("ime-english",control="single")
        actual=committed["text"][4:-6]
        self.wait(lambda:self.inline_ready(actual),"committed IME text becomes query",8)
        self.n("selection",control="single",start=4,length=len(actual.encode("utf-16-le"))//2)
        self.type_query("ec prf 0013")
        result=self.enter(expected="pre|"+PAYLOAD+" |post")
        return {"input":"installed Chinese IME driven by synthetic VK keys, not a manual physical-keyboard test", "composition_bytes":composed["ime"]["composition_bytes"],"ime_enter_submit_count":committed["enter_count"],"replacement":result}

    def test_independent_streaming(self, fallback=False):
        if not self.args.native_test:
            return "Detailed frame sampling is tested on the identical instrumented build"
        self.reset()
        if fallback:
            self.open_inline(); self.type_query("ec")
            self.f("key",self.native,self.native_title,117)
        else:
            subprocess.run([str(self.args.executable),"--history"],env=self.env,stdin=subprocess.DEVNULL,check=True,timeout=8)
        label="Search clipboard history"
        self.wait(lambda: "ControlType.Edit | "+label in self.d("dump"),"independent search ready",12)
        self.f("activate-owned",self.echo,TITLE,label)
        self.f("hotkey",self.echo,TITLE,"Ctrl+A");self.f("key",self.echo,TITLE,8)
        self.wait(lambda:self.metrics()["ready"] and self.metrics()["query"]=="","independent baseline",12)
        trace="fallback-search-trace" if fallback else "startup-search-trace"
        self.control_request("trace_begin",file=trace)
        progress=[];typed=""
        try:
            for part in ("e","c"," ","p","rf"," ","0013"):
                self.f("text",self.echo,TITLE,part);typed+=part
                m=self.wait(lambda: v if (v:=self.metrics())["query"]==typed and v["ready"] and not v["loading"] else None,"independent query update",12)
                progress.append({"query":typed,"rows":m["snapshot_model_count"]})
                time.sleep(.10)
        finally:
            self.control_request("trace_end")
        frames=json.loads((self.evidence/trace/"frames.json").read_text(encoding="utf-8"))
        if len(frames)<10 or any(f["stale"] or not f["visible"] or f["rows"]==0 or f["selected_rows"]!=1 for f in frames):
            raise RuntimeError("Independent search blanked rows or action selection during typing")
        self.shot(trace+"-final"); self.d("close")
        return {"frames":len(frames),"blank_frames":0,"progress":progress}

    def test_fuzzy_words(self):
        self.reset(); self.open_inline()
        self.type_query("ec prf 0013")
        self.confirm_ready()
        before=self.metrics()
        if before["snapshot_model_count"]!=1 or before.get("highlighted_rows",0)!=1:
            raise RuntimeError("Multiword subsequence query did not produce one highlighted match")
        self.shot("fuzzy-multiword")
        pixels=self.launch(self.args.tools/"EchoInlineDriver.exe",["sample-actions",str(self.evidence),str(self.echo.pid),TITLE,str(self.native.pid),self.native_title],"action-pixels")
        self.wait(lambda:(self.evidence/"actions.ready").exists(),"action pixel sampler ready")
        self.control_request("trace_begin",file="action-trace")
        progress=[]
        try:
            for part in (" "," ","r"," ","e"):
                self.f("text",self.native,self.native_title,part)
                raw=self.state()["text"][4:-6]
                m=self.wait(lambda:self.inline_ready(raw),"space-delimited fuzzy query")
                self.confirm_ready()
                if m["snapshot_model_count"]!=1 or m["selection"]!=before["selection"]:
                    raise RuntimeError("Refining query lost the existing matching item")
                dump=self.d("dump")
                for line in dump.splitlines():
                    if " | Copy item | " in line or " | Insert item | " in line:
                        if "enabled=False" in line: raise RuntimeError("Action icon appearance was disabled while refining")
                progress.append({"query":raw,"rows":m["snapshot_model_count"],"highlighted":m.get("highlighted_rows")})
                time.sleep(.10)
        finally:
            self.control_request("trace_end")
            (self.evidence/"actions.stop").write_text("stop",encoding="ascii")
            pixels.wait(timeout=15)
        sample=json.loads((self.evidence/"action-pixels.log").read_text(encoding="utf-8-sig"))
        if pixels.returncode!=0 or sample.get("status")!="PASS": raise RuntimeError("Action pixel capture failed")
        physical=sample["value"]["frames"]
        if len(physical)<10: raise RuntimeError("Insufficient action pixel samples")
        base=physical[0]["colors"]
        flicker=[f for f in physical if sum(max(abs(((c>>k)&255)-((d>>k)&255)) for k in (0,8,16))>15 for c,d in zip(f["colors"],base))>len(base)*.12]
        atomic(self.evidence/"action-pixel-analysis.json",{"frames":len(physical),"changed_frames":len(flicker)})
        if flicker: raise RuntimeError("Copy action pixels faded, blinked or moved during the same-item query")
        frames=json.loads((self.evidence/"action-trace/frames.json").read_text(encoding="utf-8"))
        if len(frames)<12: raise RuntimeError("Insufficient action-state samples")
        if any(f["stale"] or not f["visible"] or f["rows"]!=1 or f["selected_rows"]!=1 or f["busy"] for f in frames):
            raise RuntimeError("Pending fuzzy queries cleared selection, buttons or rows")
        self.shot("fuzzy-refined")
        final=self.enter(expected="pre|"+PAYLOAD+" |post")
        return {"frames":len(frames),"progress":progress,"replacement":final}

    def test_empty_rich(self):
        self.browser_reset("ai")
        self.f("invoke-control",self.browser,self.browser_title,"Reset empty rich composer")
        self.wait(lambda:self.browser_state()["ai"]["text"]=="","empty paragraph reset")
        self.browser_open()
        typed=""
        progress=[]
        for part in ("e","c"," ","p","rf"," ","0013"):
            self.f("text",self.browser,self.browser_title,part);typed+=part
            def current():
                m=self.inline_ready()
                return m if m and m["query"].replace("\u00a0"," ")==typed else None
            m=self.wait(current,"empty composer first-key and multiword continuity",8)
            progress.append({"query":typed,"rows":m["snapshot_model_count"]})
            if not self.browser_state()["ai"]["focused"]: raise RuntimeError("Typing focus left the composer")
        self.confirm_ready()
        if self.metrics()["snapshot_model_count"]!=1 or self.metrics().get("highlighted_rows",0)!=1:
            raise RuntimeError("Empty composer did not narrow and highlight a fuzzy match")
        self.shot("empty-rich-fuzzy")
        self.f("key",self.browser,self.browser_title,13)
        self.wait(lambda:not self.d("exists"),"empty composer replacement acknowledged",8)
        state=self.browser_state()["ai"]
        if state["text"]!=PAYLOAD or state["submitted"]!=0:
            raise RuntimeError("First-character recovery replaced or submitted the wrong content")
        return {"progress":progress,"state":state}

    def test_streaming_filter(self):
        if not self.args.native_test: return "Frame capture restricted to native-test; delivery is tested separately"
        self.reset(); self.open_inline()
        if self.args.renderer == "femtovg-wgpu":
            self.wait(lambda: self.metrics()["graphics"]["stats"][1] >= 2,"side cards ready before trace",8)
        initial=self.metrics()
        pixels=self.launch(self.args.tools/"EchoInlineDriver.exe",["sample-headers",str(self.evidence),str(self.echo.pid),TITLE,str(self.native.pid),self.native_title],"header-samples")
        self.wait(lambda:(self.evidence/"pixels.ready").exists(),"physical header sampler ready")
        self.control_request("trace_begin",file="typing-trace")
        progress=[]
        try:
            typed=""
            for part in ("e","cho","-perf","-text","-00","13"):
                self.f("text",self.native,self.native_title,part);typed+=part
                m=self.wait(lambda:self.inline_ready(typed),"incremental query " + typed,8)
                progress.append({"query":typed,"rows":m["snapshot_model_count"],"height":m["panel"][3]})
                time.sleep(.09)
            self.confirm_ready()
            time.sleep(.15)
        finally:
            self.control_request("trace_end")
            (self.evidence/"pixels.stop").write_text("stop",encoding="ascii")
            pixels.wait(timeout=15)
        samples=json.loads((self.evidence/"header-samples.log").read_text(encoding="utf-8-sig"))
        if pixels.returncode or samples.get("status") != "PASS": raise RuntimeError("Physical header sampling failed: "+str(samples))
        physical=samples["value"]["frames"]
        if len(physical)<20: raise RuntimeError("Insufficient physical display samples")
        baseline=physical[0]["colors"]
        def differs(c,d): return max(abs(((c>>i)&255)-((d>>i)&255)) for i in (0,8,16))>3
        flashes=[f for f in physical if sum(differs(c,d) for c,d in zip(f["colors"],baseline))>=7]
        atomic(self.evidence/"physical-header-analysis.json",{"frames":len(physical),"changed_header_frames":len(flashes)})
        if flashes: raise RuntimeError("Opaque header disappeared or flashed on the physical display")
        frames=json.loads((self.evidence/"typing-trace/frames.json").read_text(encoding="utf-8"))
        if len(frames)<12: raise RuntimeError("Insufficient rendering samples")
        if any(not f["visible"] or f["stale"] or f["rows"]==0 or f["navigation_busy"] for f in frames):
            raise RuntimeError("A query hid/blanked the existing panel")
        if any("capture_error" in f for f in frames): raise RuntimeError("Renderer snapshot failed")
        if self.args.renderer == "femtovg-wgpu" and any(not f["flow_enabled"] for f in frames):
            raise RuntimeError("Typing cleared the existing side-card scene")
        final=self.metrics()
        if final["snapshot_model_count"] != 1 or final["panel"][3] >= initial["panel"][3]:
            raise RuntimeError("The exact result did not narrow and shrink the panel")
        if self.state()["text"] != "pre|"+QUERY+" |post": raise RuntimeError("Typing left the original composer")
        self.shot("streaming-final")
        self.f("key",self.native,self.native_title,27)
        self.wait(lambda:not self.d("exists"),"streaming test cancelled")
        return {"frames":len(frames),"physical_display_samples":len(physical),"renderer_snapshots":sum("png" in f for f in frames),"blank_frames":0,"progress":progress}

    def test_first(self):
        state=self.reset(); atomic(self.evidence / "native-input-capabilities.json", {"ime":state["fields"]["single"]["ime"],"patterns":self.f("patterns",self.native,self.native_title,"Inline fixture single")}); m = self.open_inline(); self.shot("01-inline-empty-query")
        return {"input_foreground": True, "deactivations": self.n("state")["deactivations"], "metrics": m if self.args.native_test else None}

    def test_height(self):
        before = self.f("geometry", self.echo, TITLE)
        self.type_query(); self.shot("02-inline-filtered")
        self.wait(lambda: self.f("geometry", self.echo, TITLE)["window"][3] - self.f("geometry", self.echo, TITLE)["window"][1] < before["window"][3] - before["window"][1], "height shrank")
        after = self.f("geometry", self.echo, TITLE)
        if self.state()["text"] != "pre|" + QUERY + " |post":
            raise RuntimeError("Typing did not stay in the composer")
        return {"before": before, "after": after, "query": QUERY}

    def test_input_placement(self):
        evidence=[]
        for lower in (False,True):
            self.reset()
            g=self.f("geometry",self.native,self.native_title)
            work=g["work"]; caret=g["caret"]
            if not caret: raise RuntimeError("Owned native caret unavailable")
            desired=work[3]-140 if lower else work[1]+280
            self.f("move",self.native,self.native_title,g["window"][0],g["window"][1]+desired-caret[1])
            self.n("reset",control="single",text="pre| |post",start=4,length=0)
            g=self.f("geometry",self.native,self.native_title);caret=g["caret"]
            self.open_inline()
            scale=g.get("monitor_dpi",96)/96.0
            card=self.wait(lambda: c if (c:=self.f("card",self.echo,TITLE,"History space"))[3]-c[1]>=480*scale else None,
                           "full-size suggestions use available side",8)
            gap=caret[1]-card[3] if lower else card[1]-caret[3]
            if abs(gap-8*scale)>5:
                raise RuntimeError("Popup detached from measured caret: " + repr((caret,card,gap)))
            if not self.n("state")["foreground"]:
                raise RuntimeError("Popup took input focus")
            evidence.append({"lower":lower,"caret":caret,"card":card,"gap":gap})
            self.shot("placement-lower" if lower else "placement-upper")
            self.type_query()
            smaller=self.f("card",self.echo,TITLE,"History space")
            edge=(3 if lower else 1)
            if abs(smaller[edge]-card[edge])>5:
                raise RuntimeError("Shrinking results moved the anchored edge")
            self.f("key",self.native,self.native_title,27)
            self.wait(lambda:not self.d("exists"),"placement case dismissed")
        self.f("move",self.native,self.native_title,480,300)
        return evidence

    def test_held(self):
        self.reset(); self.open_inline(); self.type_query()
        return self.enter(held=True, expected="pre|" + PAYLOAD + " |post")

    def test_empty(self):
        self.reset(); self.open_inline(); self.type_query("not-found-echo-987654321")
        self.f("key", self.native, self.native_title, 13)
        time.sleep(0.12)
        state = self.state()
        if state["enter_count"] != 0 or not self.d("exists") or state["text"] != "pre|not-found-echo-987654321 |post":
            raise RuntimeError("Empty results leaked Enter or changed text")
        self.shot("03-inline-no-results")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "Esc dismissed")
        return state

    def test_fast(self):
        self.reset(); self.open_inline()
        self.f("text-enter", self.native, self.native_title, "not-found-fast-query")
        time.sleep(0.2)
        state = self.state()
        if state["enter_count"] or state["text"] != "pre|not-found-fast-query |post":
            raise RuntimeError("An old result or Enter was delivered during a fast query change")
        return state

    def test_unicode(self):
        self.reset(); self.open_inline(); self.type_query("邮箱🙂")
        self.f("key", self.native, self.native_title, 8)
        def coherent_backspace():
            actual=self.state()["text"][4:-6]
            return (actual if len(actual.encode("utf-16-le")) < 8 and self.inline_ready(actual) else None)
        actual=self.wait(coherent_backspace, "Unicode backspace matches actual provider text")
        self.f("key", self.native, self.native_title, 27)
        self.wait(lambda: not self.d("exists"), "Unicode query cancelled")
        if self.state()["text"] != "pre|" + actual + " |post":
            raise RuntimeError("Esc removed typed query")
        return {"actual_query": actual, "submit_count": self.state()["enter_count"]}

    def test_selection(self):
        self.reset(text="pre|" + QUERY + " |post", start=4, length=len(QUERY))
        self.open_inline(QUERY)
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_pasted_query(self):
        self.reset(); self.open_inline()
        self.f("paste-text", self.native, self.native_title, QUERY)
        self.wait(lambda: self.inline_ready(QUERY), "pasted query filtered")
        return self.enter(expected="pre|" + PAYLOAD + " |post")

    def test_arrows(self):
        self.reset(); self.open_inline(); before = self.state()
        m1 = self.metrics() if self.args.native_test else None
        self.f("key", self.native, self.native_title, 40)
        time.sleep(0.1); after = self.state()
        if (before["start"], before["length"], before["text"]) != (after["start"], after["length"], after["text"]):
            raise RuntimeError("Arrow key changed the original input instead of the suggestion")
        if self.args.native_test and m1["selection"] == self.metrics()["selection"]:
            raise RuntimeError("Arrow did not move suggestion highlight")
        self.f("hotkey", self.native, self.native_title, "Alt+V")
        self.wait(lambda: not self.d("exists"), "second Alt+V dismissed")
        return {"caret_preserved": True, "second_hotkey_dismissed": True}

    def test_outside(self):
        self.reset(); self.open_inline()
        self.f("key", self.native, self.native_title, 37)
        self.wait(lambda: not self.d("exists"), "caret left owned range")
        return self.state()

    def test_control(self, control):
        self.reset(control); self.open_inline(); self.type_query()
        return self.enter(control, expected="pre|" + PAYLOAD + " |post")

    def test_password(self):
        for control in ("password", "readonly"):
            before = self.reset(control)["fields"][control]["text"]
            self.f("hotkey", self.native, self.native_title, "Alt+V")
            self.wait(lambda: self.d("exists"), "protected-input fallback")
            self.wait(lambda: "Input stays active" in self.d("dump"), "explicit unavailable-input notice")
            if "ControlType.Edit | Search clipboard history" in self.d("dump"):
                raise RuntimeError("An unsupported input unexpectedly activated an Echo search field")
            if not self.n("state")["foreground"]:
                raise RuntimeError("An unsupported input lost foreground to Echo")
            if self.args.native_test and self.metrics()["quick_insert"]["has_target"]:
                raise RuntimeError("Protected input incorrectly admitted a paste target")
            if self.state(control)["text"] != before:
                raise RuntimeError("Protected input was changed")
        return "Password and read-only controls remain focused; unavailable replacement never reveals an activating search field"

    def test_f6(self):
        self.reset(); self.open_inline(); self.type_query()
        self.f("key", self.native, self.native_title, 117)
        self.wait(lambda: "ControlType.Edit | Search clipboard history" in self.d("dump"), "independent search field")
        if self.state()["text"] != "pre|" + QUERY + " |post":
            raise RuntimeError("F6 deleted the original query")
        return "Typed query preserved; normal Echo search regained focus explicitly"

    def browser_state(self):
        return json.loads(self.f("read-control", self.browser, self.browser_title, "Browser fixture state"))

    def browser_reset(self, control="search"):
        if self.d("exists"):
            self.d("close")
            self.wait(lambda: not self.d("exists"), "Echo hidden before browser reset")
        label = {"search": "Reset search input", "textarea": "Reset textarea composer", "ai": "Reset rich composer"}[control]
        # Only an owned, visible, unobscured control is clicked. The native-test
        # browser is raised temporarily and normal Z order is restored afterwards.
        if not self.f("geometry", self.browser, self.browser_title)["foreground"]:
            self.f("activate-owned", self.browser, self.browser_title, "Echo Inline Composer Fixture")
        self.f("english-owned", self.browser, self.browser_title)
        self.f("invoke-control", self.browser, self.browser_title, label)
        state = self.wait(lambda: (s if s[control]["focused"] else None) if (s := self.browser_state()) else None,
                          "owned browser input focused", 8)
        self.wait(lambda: self.f("geometry", self.browser, self.browser_title)["foreground"], "browser foreground")
        return state

    def browser_open(self, query=""):
        self.f("hotkey", self.browser, self.browser_title, "Alt+V")
        result = self.wait(lambda: self.inline_ready(query), "browser inline session", 12)
        if not self.f("geometry", self.browser, self.browser_title)["foreground"]:
            raise RuntimeError("Inline popup activated instead of retaining browser input focus")
        return result

    def browser_query(self, query=QUERY):
        self.f("text", self.browser, self.browser_title, query)
        return self.wait(lambda: self.inline_ready(query), "browser query filtered", 10)

    def browser_enter(self, control="search", held=False, expected=None):
        self.confirm_ready()
        self.f("held-enter" if held else "key", self.browser, self.browser_title, *([] if held else [13]))
        self.wait(lambda: not self.d("exists"), "browser replacement acknowledged", 10)
        state = self.browser_state()
        if state[control]["submitted"] != 0:
            raise RuntimeError("Enter submitted the browser input")
        if expected is not None and state[control]["text"] != expected:
            raise RuntimeError(f"Browser replacement incorrect: {state[control]['text']!r}")
        if not state["mention"] or state["attachment"] != "synthetic.pdf":
            raise RuntimeError("Inline replacement destroyed a mention or attachment")
        return state

    def browser_checks(self):
        if self.echo is None:
            self.start_processes()
        base = Path(os.environ["ProgramFiles"])
        browser = base / "Google/Chrome/Application/chrome.exe" if self.args.browser == "chrome" else Path(os.environ.get("ProgramFiles(x86)", str(base))) / "Microsoft/Edge/Application/msedge.exe"
        if not browser.exists():
            raise RuntimeError("Requested browser is not installed: " + str(browser))
        profile = self.evidence / "isolated-browser-profile"
        html = (self.root / "tests/native/fixtures/inline-composer.html").as_uri()
        self.browser = self.launch(browser, ["--user-data-dir=" + str(profile), "--no-first-run", "--no-default-browser-check",
            "--disable-background-mode", "--disable-background-networking", "--disable-sync", "--disable-extensions",
            "--force-renderer-accessibility", "--window-size=1100,1000", "--window-position=600,150", html if self.args.omnibox else "--app=" + html], "browser")
        def title():
            command = f"$p=Get-Process -Id {self.browser.pid}; $p.MainWindowTitle | ConvertTo-Json -Compress"
            result = subprocess.run(["pwsh", "-NoProfile", "-Command", command], capture_output=True, stdin=subprocess.DEVNULL,
                                    text=True, encoding="utf-8", timeout=8)
            value = json.loads(result.stdout.lstrip("\ufeff")) if result.returncode == 0 else ""
            return value if value and "Echo Inline Composer Fixture" in value else None
        self.browser_title = self.wait(title, "owned browser fixture window", 15)
        self.record(self.browser, self.browser_title)
        (self.evidence/"browser-initial-tree.txt").write_text(self.f("dump-tree",self.browser,self.browser_title),encoding="utf-8")
        if self.args.omnibox:
            self.f("activate-owned",self.browser,self.browser_title,"Echo Inline Composer Fixture")
        self.wait(lambda: self.browser_state(), "local browser accessibility tree ready", 15)
        if self.args.omnibox:
            self.check("browser-omnibox-inline-query-keeps-popup-and-never-navigates", self.test_omnibox)
        self.check("browser-empty-paragraph-first-key-spaces-and-replacement", self.test_empty_rich)
        self.check("browser-search-caret-query-exact-replacement", lambda: self.test_browser_control("search"))
        self.check("browser-textarea-composer-exact-replacement", lambda: self.test_browser_control("textarea"))
        self.check("browser-rich-ai-composer-preserves-mention-and-attachment", lambda: self.test_browser_control("ai"))
        self.check("browser-held-enter-does-not-send", self.test_browser_held)
        self.check("browser-empty-results-never-submit", self.test_browser_empty)
        self.check("browser-fast-query-enter-never-uses-stale-row", self.test_browser_fast)
        self.check("browser-undo-is-a-local-replacement", self.test_browser_undo)
        self.check("browser-click-suggestion-does-not-take-focus", self.test_browser_click)
        self.check("browser-profile-cleanup", self.close_browser)

    def test_omnibox(self):
        # This is a NORMAL owned Chrome window, not --app (which has no omnibox).
        self.browser_reset()
        self.f("hotkey", self.browser, self.browser_title, "Ctrl+L")
        self.f("key", self.browser, self.browser_title, 8)
        before_dump = self.f("dump", self.browser, self.browser_title)
        (self.evidence / "omnibox-before.uia.txt").write_text(before_dump, encoding="utf-8")
        edits = [line.split(" | ")[1] for line in before_dump.splitlines()
                 if line.startswith(("ControlType.Edit | ", "ControlType.ComboBox | ")) and ("Address" in line or "address" in line or "鍦板潃" in line)]
        if len(edits) != 1:
            raise RuntimeError("Could not identify the owned browser address bar uniquely")
        label=edits[0]
        self.wait(lambda:self.f("read-edit",self.browser,self.browser_title,label)=="", "empty address bar before activation")
        bounds=self.f("card",self.browser,self.browser_title,label)
        atomic(self.evidence/"omnibox-range-before.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.browser_open()
        atomic(self.evidence/"omnibox-range-open.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.f("text",self.browser,self.browser_title,QUERY)
        atomic(self.evidence/"omnibox-range-typed.json",self.tool("owned_text_probe.exe",[str(self.evidence),str(self.browser.pid)]))
        self.wait(lambda:self.inline_ready(QUERY),"omnibox query filtered",10)
        self.wait(lambda: self.metrics()["inline"]["readiness"][7] == 1,
                  "omnibox query and result synchronized",5)
        card=self.f("card",self.echo,TITLE,"History space")
        if card[1] < bounds[3]-3:
            raise RuntimeError("A top address bar must expand below rather than cover or compress against it")
        if card[1]-bounds[3] > 48:
            raise RuntimeError("Echo is detached from the address bar")
        self.shot("omnibox-filtered")
        self.f("key", self.browser, self.browser_title, 13)
        self.wait(lambda:not self.d("exists"),"omnibox replacement acknowledged",10)
        actual=self.f("read-edit",self.browser,self.browser_title,label)
        if actual != PAYLOAD:
            raise RuntimeError("Address bar replacement mismatch: " + repr(actual))
        state=self.browser_state()
        if any(state[c]["submitted"] for c in ("search","textarea","ai")):
            raise RuntimeError("Completion submitted the local fixture")
        # A successfully queried page state proves Enter did not navigate away.
        return {"text":actual,"original_page_still_loaded":True,"input_bounds":bounds,"card":card}

    def test_browser_control(self, control):
        self.browser_reset(control)
        before = self.browser_open()
        self.browser_query()
        self.shot("browser-" + control + "-filtered")
        expected = ("@Teammate pre|" if control == "ai" else "pre|") + PAYLOAD + " |post"
        after = self.browser_enter(control, expected=expected)
        return {"control": control, "provider": before["inline"]["provider"] if self.args.native_test else "capability-checked", "state": after}

    def test_browser_held(self):
        self.browser_reset("ai"); self.browser_open(); self.browser_query()
        return self.browser_enter("ai", held=True, expected="@Teammate pre|" + PAYLOAD + " |post")

    def test_browser_empty(self):
        self.browser_reset(); self.browser_open(); self.browser_query("no-result-inline-123987")
        self.f("key", self.browser, self.browser_title, 13)
        state = self.browser_state()
        if state["search"]["submitted"] or state["search"]["text"] != "pre|no-result-inline-123987 |post" or not self.d("exists"):
            raise RuntimeError("No-result Enter changed or submitted the search field")
        self.f("key", self.browser, self.browser_title, 27)
        self.wait(lambda: not self.d("exists"), "browser Esc cancelled")
        return state

    def test_browser_fast(self):
        self.browser_reset("ai"); self.browser_open()
        self.f("text-enter", self.browser, self.browser_title, "no-result-fast-123987")
        time.sleep(0.2); state = self.browser_state()
        if state["ai"]["submitted"] or state["ai"]["text"] != "@Teammate pre|no-result-fast-123987 |post":
            raise RuntimeError("Fast typing submitted the AI composer or replaced an old result")
        return state

    def test_browser_undo(self):
        self.browser_reset(); self.browser_open(); self.browser_query(); self.browser_enter(expected="pre|" + PAYLOAD + " |post")
        self.f("hotkey", self.browser, self.browser_title, "Ctrl+Z")
        state = self.wait(lambda: (s if s["search"]["text"] == "pre|" + QUERY + " |post" else None) if (s := self.browser_state()) else None,
                          "one Undo restores query without resetting composer")
        return state

    def test_browser_click(self):
        self.browser_reset(); self.browser_open(); self.browser_query()
        self.f("click-owned", self.echo, TITLE, "Insert item")
        self.wait(lambda: not self.d("exists"), "mouse-selected replacement")
        state = self.browser_state()
        if not self.f("geometry", self.browser, self.browser_title)["foreground"] or state["search"]["text"] != "pre|" + PAYLOAD + " |post" or state["search"]["submitted"]:
            raise RuntimeError("Mouse completion stole focus, submitted, or replaced the wrong text")
        return state

    def close_browser(self):
        if self.d("exists"):
            self.d("close"); self.wait(lambda: not self.d("exists"), "Echo hidden for browser cleanup")
        if self.browser and self.browser.poll() is None:
            self.f("close-owned", self.browser, self.browser_title)
            self.browser.wait(timeout=10)
        return "Only the isolated test browser was closed; daily profile was not used"

    def cleanup(self):
        errors = []
        if self.browser and self.browser.poll() is None:
            try:
                self.close_browser()
            except Exception:
                pass
        if self.native and self.native.poll() is None:
            try:
                self.n("quit"); self.native.wait(timeout=8)
            except Exception as error:
                errors.append("native fixture cleanup: " + str(error))
        if self.echo and self.echo.poll() is None:
            try:
                secondary = subprocess.run([str(self.args.executable), "--quit"], env=self.env, capture_output=True, timeout=8)
                if secondary.returncode != 0:
                    raise RuntimeError("Quit handoff failed")
                self.echo.wait(timeout=8)
            except Exception as error:
                errors.append("Echo cleanup: " + str(error))
        for error in errors:
            self.checks.append({"name": "cleanup", "status": "FAIL", "error": error})
        for log in self.logs:
            log.close()
        return errors

def main() -> int:
    parser = argparse.ArgumentParser()
    for name in ("root", "executable", "tools", "template", "evidence"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--renderer", choices=("software", "femtovg-wgpu"), default="femtovg-wgpu")
    parser.add_argument("--native-test", action="store_true")
    parser.add_argument("--browser-only", action="store_true")
    parser.add_argument("--omnibox", action="store_true")
    parser.add_argument("--only", default="")
    parser.add_argument("--browser", choices=("none", "chrome", "edge"), default="none")
    args = parser.parse_args()
    if os.environ.get("ECHO_WINDOWS_ACCEPTANCE") != "1":
        raise RuntimeError("Explicit acceptance authorization required")
    print("START inline acceptance", datetime.now(timezone.utc).isoformat(), flush=True)
    run = Run(args)
    success = False
    try:
        if not args.browser_only:
            run.native_checks()
        if args.browser != "none":
            run.browser_checks()
        success = True
    except Exception as error:
        print("FAIL", str(error), flush=True)
    finally:
        if run.cleanup():
            success = False
        atomic(run.evidence / "summary.json", {
            "schema": "echo.inline.acceptance.v1", "status": "PASS" if success else "FAIL",
            "started": run.started, "finished": datetime.now(timezone.utc).isoformat(), "checks": run.checks,
            "limitations": [
                {"name": "physical-ime", "status": "NOT_RUN", "reason": "Synthetic input is not physical Chinese IME evidence"},
                {"name": "physical-mixed-dpi", "status": "NOT_RUN", "reason": "Only the actual display configuration is tested"},
                {"name": "all-composers", "status": "NOT_RUN", "reason": "Capability-based behavior is not a universal application certification"},
            ],
        })
    return 0 if success else 1

if __name__ == "__main__":
    raise SystemExit(main())
