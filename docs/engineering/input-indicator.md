# Input method indicator

The passive badge observes the current writable input independently of Quick Insert.
Appearance settings contains **Input method indicator**, enabled by default. The
switch edits the settings draft; Save persists the choice and starts/stops observation.
Cancel keeps the saved choice. No schema migration is required for the new defaulted
`UiSettings.input_method_indicator` field.

## Ownership and behavior

- Engine exports content-free input mode, composition state and geometry samples.
- Windows owns one event/message worker, bounded focus/UIA queries and the shared
  target-thread observer. Status-only protocol requests never copy preedit text.
  The existing Quick Insert protocol remains separate from status-only replies.
- Presentation decides freshness, suppression and 48-by-36-DIP badge placement.
  The label uses 18-DIP text, an 8-DIP radius and an 8-DIP caret gap.
- Desktop owns the Slint badge, expiry timer and theme binding. Its software frame
  is separate from the main card's buffer and frame-commit acknowledgements.

Foreground/focus events invalidate samples. Mode is refreshed every 100ms while
an input is valid; geometry events refresh the anchor and periodic revalidation
checks identity/writability. Failed reads back off. Disabled observation releases
hooks and stops sampling. Stale, unknown, composing, hidden, password and readonly
targets do not show a badge. Window-only anchors are rejected.

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

This prints only target identity, mode and geometry changes. Verify physical Shift,
Win+Space, candidate visibility, typing, scrolling, application transitions and
mixed-DPI monitors separately. Mark scenarios not performed as NOT_RUN.

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

`tests/native/Invoke-TerminalPlainPasteAcceptance.ps1` runs isolated CMD/PowerShell
readers through the clipboard preservation wrapper. The test checks actual received
text and rejects a stale process identity. Warp and Windows Terminal require their
own native acceptance; these fixtures do not establish support for their UI trees.
