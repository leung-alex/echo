# Alt+V rendering and measurement

The main card remains live Slint content. Side-card snapshots validate space and
data revision, query, display state, dimensions, actual raster scale, theme,
selection, scrolling and thumbnails. Moving the window or changing expansion
direction only changes projection. Normal activation invalidates the scene while
retaining valid textures and the keyed offscreen row model. Existing texture/card
budgets, eviction, enabled 30-second hidden reclamation, device reset and shutdown
still govern resource release.

The main image cache remembers at most eight recent viewport request identities
after freeing its pixels. On activation, matching rows in the retained query can
request those thumbnails immediately, overlapping storage reads with window
preparation. Requests still use the normal image epoch, pending deduplication and
memory budget; this does not preload images while the window is hidden.

Current visible neighbors bypass speculative navigation's 120 ms prewarm timer.
On Alt+V, already available side-card content can prepare at the expected popup
size while the independent Windows input worker verifies the target. The HWND
stays hidden and unmoved. Temporary Slint properties are restored before returning
to the event loop, and normal scene preparation validates the actual query,
dimensions, DPI and revision again. No target decision uses these pixels.
The first presented window region is tied to session, query, content and scene
versions. The popup stays cloaked until its matching frame is submitted and DWM
synchronization completes. Cached rendering never authorizes a paste target.

Two pinned FemtoVG 0.25.1 fixes retain font cache identity and keep content
pipelines through Slint's separate background-clear flushes. Slint 1.17.1's WGPU
surface requests one queued frame while retaining vsync. See the corresponding
vendor patch notes. Repeated activation also preserves unchanged native DWM
theme/chrome; actual theme notifications invalidate that state.

DEV optimizes third-party dependencies at level 3. Echo workspace crates retain
their normal DEV configuration. This profile adjustment was explicitly approved
after source-only measurements, and its benefit must be reported separately.

## Native latency observer

`tests/native/Measure-PopupTiming.py` uses an owned WinForms Edit or a local HTML
textarea in an isolated Edge profile. It sends guarded Alt+V and Escape, never
paste or Enter. A fresh evidence directory must contain the measured executable
as `echo-timing.exe`, the existing `EchoInlineDriver.exe` and
`EchoInlineFixture.exe` native test tools, and a `data` directory with a
capture-disabled synthetic fixture marker. Normal clipboard history is not a
benchmark fixture. Stop the normal Echo instance before taking the hotkey lease.

```powershell
python tests/native/Measure-PopupTiming.py <fresh-evidence-root> --target native --profile dev --first-count 10 --repeat-count 30 --sampler dxgi
python tests/native/Analyze-PopupTiming.py <native-evidence-root> <browser-evidence-root>
```

Use a separate root for each target, build, control or repetition. Record the
executable SHA256 and exact source/configuration. Startup follows the current DEV
path: show the manager, wait for content and hotkey registration, hide it, then
send the first Alt+V. This measures the first popup in a fresh process, not process
creation time or an uninitialized tray-only renderer.

GDI capture is built into the driver. The optional DXGI observer requires Python
`numpy`, `Pillow`, `psutil`, `opencv-python`, `dxcam==0.3.0` and
`comtypes==1.4.16`; install them in an isolated environment. Its scoped COM
ownership correction prevents dxcam from explicitly releasing a reference that
comtypes already owns. Neither observer changes Echo's rendering configuration.

Both observers save desktop pixels, the guarded SendInput QPC origin and each
capture's start/end. The ROI must contain the entire projected cards; analysis
rejects clipped samples. Detection requires the main heading and body details,
side surface and side text to match the settled frame. Report each visibility
interval and use its upper bound for acceptance, plus median, P95, maximum and
sampling interval. DXGI polling frequency is distinct from desktop refresh rate;
desktop presentation QPC values are saved separately. Internal timing and a
`--no-pixels` observer control are supplementary evidence, not visible latency.

Supplement with `--idle-seconds 31`, a larger synthetic corpus, query changes,
pointer navigation, and `--renderer software`. Software mode intentionally has
one flat card and needs separate visual assessment. Automated pixels and input
do not certify physical keyboard/IME or mixed-DPI hardware behavior.
