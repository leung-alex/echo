# Input method indicator

The passive badge observes the current writable input independently of Quick Insert.
General settings contains **Input method indicator**, enabled by default. The
switch edits the settings draft; Save persists the choice and starts/stops observation.
Cancel keeps the saved choice. No schema migration is required for the new defaulted
`UiSettings.input_method_indicator` field.

## Ownership and behavior

- Engine exports content-free input mode, composition state and geometry samples.
- Windows owns one event/message worker, bounded focus/UIA queries and the shared
  target-thread observer. Status-only protocol requests never copy preedit text.
  The existing Quick Insert protocol remains separate from status-only replies.
- Caret geometry has an independent `Echo.CaretObservation.v1` mailbox. In
  `primary` mode a target-thread read-only TSF session selects a fresh collapsed
  `GetTextExt` rectangle before validated NativeCaret/UIA/MSAA/adjacent candidates.
  The observer uses the target's existing TSF manager and a fixed client CLSID;
  it never activates TSF, reads text, changes selection, or changes input mode.
  `shadow` collects the new provider while displaying the legacy choice, and
  `legacy` disables the new geometry session. The process-start diagnostic
  variable `ECHO_CARET_PROVIDER` defaults to `legacy` until the native, physical
  and Release gates are complete; invalid values fail monitor startup.
  Cross-process observers are denied for the Windows Codex host (`ChatGPT.exe`)
  and `codex.exe`. These WebView2/TSF processes can tear down their input stack
  during Settings navigation while a third-party `WH_CALLWNDPROC` callback is
  in flight; the indicator stays on its legacy geometry path for those targets.
  For the mode half of the badge, the worker may query the already-owned target
  thread's existing IME window with bounded `WM_IME_CONTROL` reads. This does not
  load a module, install a hook, read text, or authorize an insertion; if the
  thread, process instance, foreground root, or conversion evidence cannot be
  verified, the mode remains Unknown and the badge stays hidden.
- Presentation decides freshness, suppression and 48-by-36-DIP badge placement.
  The label uses 18-DIP text, an 8-DIP radius and an 8-DIP caret gap.
- The badge stays black with bold white text regardless of the Echo theme. Chinese
  uses Microsoft YaHei UI; English uses Segoe UI Variable Text.
- Desktop owns the Slint badge and expiry timer. Its software frame
  is separate from the main card's buffer and frame-commit acknowledgements.

Foreground/focus events invalidate samples. Mode is refreshed every 100ms while
an input is valid; geometry events and a 100ms revalidation cadence refresh the
anchor before the 150ms geometry TTL expires. Only a validated provider result
updates the geometry timestamp; mode heartbeats never rejuvenate an old anchor.
Hosted probes are coalesced to one pending query. During same-target geometry
revalidation, an already visible badge keeps its current frame for at most 250ms;
the desktop side does not move or re-show that frame. A stale geometry sample is
never published as a new coordinate, and the badge hides once the hold expires or
the target identity changes, until a fresh result arrives. Failed reads back off.
Disabled observation releases hooks and stops sampling. Stale, unknown-mode, hidden,
password and readonly targets do not show a new badge. Generic window-only anchors are
rejected. In `legacy`, a `Control`/`Window` rectangle is also rejected as a caret
surrogate; this keeps a whole WeChat composer from producing a badge at its
unrelated right edge. A passive badge is admitted from a focused UIA/MSAA
element only when it returns a live caret-like rectangle, or when the element
exposes a focused writable `ValuePattern` and we can derive an explicitly
estimated one-pixel editor caret from its current text end. The estimate uses
the editor's own rectangle and value, never the pointer or the whole control
rectangle, and exact native/UIA/MSAA/TSF geometry always wins. This lets rich
editors remain visible for the indicator even when they are not safe for Quick
Insert authorization. Warp has no passive pointer fallback: when its caret
provider is not precise, the badge stays hidden instead of following the mouse.
Alt+V may still use the pointer captured at activation for explicit plain-paste
placement, with the existing work-area clamping. Other applications keep their
existing placement policy.

When a primary TSF observer reaches its bounded callback capacity, it backs off
requests and keeps the last ready candidate only while its normal freshness and
target identity checks remain valid. A target or context change retires that
candidate immediately; callback capacity is never increased to hide a lifecycle
failure.

The TSF response rectangle is already in physical screen coordinates. Response
word 26 is a host-resolved marker (`0`), not a scale factor: the host resolves
the monitor, work area and effective DPI from that fresh caret rectangle using
`MonitorFromRect`. It does not apply a second DPI multiplication or revive a
fallback rectangle when TSF has no caret. The host also validates the returned
view HWND against the live input process/thread, process instance, focused view,
root ancestry and foreground before admitting the rectangle; the target's
`Allowed` marker is required as well.

WeChat's `WeChat.exe`/`Weixin.exe`/`WeChatAppEx.exe` WebView editor is
fail-closed for the passive TSF observer. Echo never injects a caret observer
into those processes, and a root `Control`/`Window` rectangle is never used as
the caret. When WeChat reports no native `hwndFocus`, the worker may use the
foreground root only for its existing IME status window and a live focused UIA
element; the known same-process `CWebviewControlHostWnd` is also queried for
`OBJID_CARET` as a bounded MSAA fallback. Exact native/UIA/MSAA rectangles are
preferred. Some WeChat WebView builds expose a focused editable UIA element but
return no collapsed text range and no MSAA caret. Echo keeps that editor visible
using the `ValuePattern` text-end estimate, with an empty editor anchored at its
content inset; it never uses the editor's far edge as a caret and never injects
a TSF observer into WeChat. A live editor identity or whole `Control`/`Window`
rectangle alone is not enough. A cached UIA editor is not reused for the
indicator after each hosted probe loses live focus, so switching to a page
without a composer hides the badge once instead of carrying the old coordinate
into the new page.

Chinese/native and alphanumeric conversion states are interpreted together with
the input language. Missing or conflicting evidence is Unknown. A keyboard layout
alone does not identify the internal Chinese/English mode of a Chinese IME.

Background startup initializes the UI when this feature is enabled so the badge
works before the first Echo activation. Closing the main window keeps observation
alive; explicit Quit retires the worker and badge. When disabled at startup, the
existing deferred-UI startup remains available.

## Verification

Run the ordinary `echo.cmd self-check`, `echo.cmd format --check`, and
`echo.cmd verify` gates. Unit/offscreen coverage checks freshness, placement,
language interpretation, invalidation, disabled lifetime, themes and DPI scaling.

For owned native acceptance:

```powershell
cargo build -p echo-desktop --features native-test --locked
$env:ECHO_WINDOWS_ACCEPTANCE = '1'
python tests/native/Invoke-InputIndicatorAcceptance.py --executable target/debug/echo-desktop.exe --evidence .local/echo/input-indicator-new-run
```

Use a new evidence directory. The runner creates capture-disabled synthetic data,
starts only owned fixture/Echo processes, captures only Echo-rendered pixels, and
checks focus, caret motion, settings, theme rendering and mode detection. Installed
Microsoft Pinyin and Doubao profiles are activated on the fixture thread only; no
session-wide/default input settings are changed. Mode switching via IMM is automated
native evidence, not physical Shift-key or Codex/browser compatibility acceptance.

Read-only physical diagnostics are also available:

```powershell
cargo run -p echo-windows --features native-test --example input_status_probe -- 30
```

The content-free caret probe accepts `--seconds 1..300`,
`--provider shadow|primary|legacy` and a new `--output <path>` file:

```powershell
cargo run -p echo-windows --features native-test --example caret_geometry_probe -- --seconds 30 --provider shadow --output <new-evidence>\geometry.jsonl
```

The output records source, confidence, sequence, context epoch and physical
rectangle only. It refuses to overwrite an existing file and does not edit
text, read the clipboard or move focus.

This prints only target identity, mode and geometry changes. Verify physical Shift,
Win+Space, candidate visibility, typing, scrolling, application transitions and
mixed-DPI monitors separately. Mark scenarios not performed as NOT_RUN.

If an owned native fixture loses the foreground, acceptance must stop and fail
closed. The runner must not reactivate the fixture after Echo starts, and the
monitor must reject observations whose PID/root HWND do not match the explicit
fixture identity. This rule was added after a prior fail-open run continued
after foreground loss and exited the user's browser; a later rerun requires a
new authorized evidence directory and must keep the affected native scenarios
NOT_RUN until they are exercised safely.

Candidate-window suppression is limited to windows near the verified caret.
Persistent IME toolbars and candidate windows next to another editor must not
hide a badge merely because they overlap the foreground application's window.
Set `ECHO_INPUT_STATUS_TRACE=1` for the native-test probe to compare raw composition
state with the candidate-window filter; no input text is logged.

## Terminal compatibility

Explicit Alt+V in classic console hosts (CMD/PowerShell), Windows Terminal and Warp
can use plain paste even when no editable UIA range is available. The host class,
process identity and captured focus are revalidated before Ctrl+V. This mode does
not read command text or replace a query range, and typing does not filter results.
It does not authorize passive observation: the badge still requires a separately
verified input position and mode. Inaccessible hosts such as Warp may therefore
accept ordinary paste without providing enough information for a badge.

Classic console observation resolves the real conhost IME thread independently
from the shell PID reported by the console HWND. A focused console document may
report read-only text while still exposing its insertion cursor; this exception
is geometry-only and does not authorize reading commands or replacing ranges.
Protocol v4 binds the dispatch HWND, input HWND, process creation time and thread.
Normal state sampling does not request geometry, text or a TSF edit session.

Terminal positioning also checks a focused TSF document's display bounds and an
explicit IMM candidate exclusion rectangle. Whole-window bounds, default floating
IME positions, missing coordinates and off-window rectangles are rejected.
The tested Warp window returned a valid mode but only whole-window TSF bounds and
no usable IMM caret: its precise popup placement and passive badge remain
unsupported in that scene. Do not report this as a successful Warp positioning
fix; the safe result is hidden rather than mouse-following.

Plain-paste popups retain their Esc/F6/Enter keyboard lease while focus remains in
the terminal. Opening Settings retires that lease and activates the manager.
Esc in Settings uses the same hide-to-tray path as its close button, including any
existing unsaved-change confirmation.

`Invoke-TerminalIndicatorAcceptance.py` checks a visible console badge, bounded
sampling without focus invalidation loops, the complete Alt+V/Esc lifecycle, and
Settings Esc using isolated data and owned windows. It does not mutate the
clipboard or establish physical IME-switch/mixed-DPI acceptance.

`tests/native/Invoke-TerminalPlainPasteAcceptance.ps1` runs isolated CMD/PowerShell
readers through the clipboard preservation wrapper. The test checks actual received
text and rejects a stale process identity. Warp and Windows Terminal require their
own native acceptance; these fixtures do not establish support for their UI trees.

## Warp TSF geometry investigation (2026-09-17)

A disposable content-free probe on the focused Warp thread successfully obtained
its existing TSF manager, document, context and read-only selection. Both synchronous
and async-permitted read-session requests returned S_OK. GetTextExt on a collapsed
local range also returned S_OK, but returned (2559,1439,2560,1439), a zero-height
rectangle outside Warp's (112,343,1392,1143) window. It is unusable as a caret.
The probe did not read text or modify the document, selection or input mode. Its
balanced client registration and edit sessions are not included in production.
Evidence is local under `.local/echo/tsf-probe/results.log`; this observation applies
to the tested Warp build/context, not every possible future Warp version.

`tests/native/Invoke-WarpCompatibilityAcceptance.py` checks the safe no-caret
behavior against an already running Warp with a separate synthetic Echo
instance. It checks that the passive badge is suppressed when only the invalid
whole-window TSF result is available, while explicit plain-paste placement keeps
its captured pointer and focus-preservation behavior. Set
`ECHO_WINDOWS_ACCEPTANCE=1` and supply `--executable` (native-test build) and a new
`--evidence` directory. Physical Shift switching, split panes and mixed-DPI remain
separate acceptance cases.

Plain-paste popups explicitly own Up/Down and Tab/Shift+Tab for Echo browsing.
This session-bound navigation lease does not fabricate IME or replacement-range
evidence, and is retired when the popup hides.

A known input mode stays visible during composition and candidate selection.
Unknown composition state alone neither hides the badge nor backs off mode sampling;
unknown input mode, stale focus and Quick Insert suppression still hide it. This
changes only the passive badge, not Quick Insert composition/key safety.


## Persistent switch diagnostics

Mode flips last 80ms (40ms per half), with unchanged software rendering and native
window geometry. Content-free diagnostics are enabled by default under
`<ECHO_DATA_DIR>/logs/input-indicator.jsonl` (normally
`%LOCALAPPDATA%/Echo/logs/input-indicator.jsonl`). Three rotated files
`input-indicator.1.jsonl` through `.3.jsonl` retain recent history; each file is
limited to approximately 1 MiB. Logs persist across restarts.

Records include UTC time, Echo PID, sequence, observation generation, target PID
and window handles, previous/target/displayed mode, animation phase, show/hide,
sample age, and suppression/foreground/placement validity when hidden. They never
contain input text, clipboard contents, or window titles. `target-changed`,
`flip-start`, `flip-midpoint`, and `flip-complete` describe state-machine transitions,
not proof that a compositor displayed each frame. `skip-hidden` and
`skip-initial-or-invalid` help investigate Codex composer auto-refocus after sending.
The observer logs changes before UI-event coalescing; this distinguishes a mode
that never arrived from one merged before delivery. It cannot record unobserved
changes between sampling intervals.

An old `InputIndicatorExpired` event may remain queued after a newer observation
has been produced. Each freshness timer carries a serial; a newer observation,
target transition, disable, or stop invalidates older serials before they can
hide the current badge. The current serial still expires the sample at the
normal freshness boundary, while transient same-target unavailability uses the
bounded retention window. This prevents a delayed timer from creating a
hide/show blink without keeping a dead sample visible indefinitely.

A bounded 128-entry queue is drained by the existing domain worker. Producers
never wait for disk or create another thread. Queue overflow is counted in the
next accepted record's `dropped` field; disk errors do not stop input. Idle samples
and individual rendered frames are not logged.

Geometry changes are part of the observation key even when the mode remains EN or
中. The records carry the selected source, confidence, request sequence, context
epoch, observed age and rectangle; a native move emits `geometry-applied` after
the passive badge receives the new PhysicalPosition. Stable coordinates remain
deduplicated, so polling does not create a per-frame idle log stream.


A delivered mode change explicitly requests a redraw of the passive badge. Without
this wake-up, Slint change handlers can wait until the next 100ms observation tick
and the native surface can miss the entire flip after hide/refocus. Native
acceptance enables `ECHO_INDICATOR_FRAME_TRACE` on its test build only, checks
actual framebuffer spans across repeated focus restoration, and measures delivery
to flip start. These test-only frame records are absent from normal builds.

The software backend also requests a fresh native frame whenever a window is
shown. A queued redraw consumed while hidden must not suppress later animation
frames. Initial reveals still settle without flipping; native assertions distinguish
those from switches delivered to an already visible badge.
