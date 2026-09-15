# Caret-anchored Quick Insert and global shortcuts

## User behavior

Open **Settings > Keyboard & insertion**. The global shortcut defaults to **Alt+V**.
The checkbox enables/disables it; the text field accepts Ctrl and/or Alt, optional
Shift, and A-Z, 0-9, Space, or F1-F24 except F12. Examples: `Alt+V`, `Ctrl+Shift+V`,
`Ctrl+Alt+J`. Windows-key shortcuts and modifier-only/plain-letter bindings are
intentionally rejected. Save applies the binding without restarting. Reset to
Alt+V edits the draft; Save commits it. Retry saved shortcut retries a failed
startup registration without modifying the stored preference.

Bare `Ctrl+V` (paste recursion), `Alt+F4`, and `Alt+Space` are reserved and rejected.

The global binding opens **Quick Insert**, not the ordinary manager. Press it
again while Echo is focused to dismiss. Esc/Hide uses the existing unsaved-draft
confirmation and clears the insertion session. Dismissal restores the original
window only when it is still the same process/window and no unrelated app has
already taken foreground. Window Close still hides; explicit Quit releases the
registration and stops the resident.

Quick Insert always uses caret anchoring when a usable anchor is available and
filters suggestions in the original input when supported. These behaviors are
fixed, not user settings. The manager/settings retain their normal layout.
Switching spaces or opening Settings inside a Quick Insert session does not
recapture Echo's own input control as the destination.

## Ownership and sequencing

- `echo-engine/global_shortcut.rs`: portable syntax, validation and canonical names.
- `echo-engine/ui_settings.rs`: defaulted JSON fields `global_hotkey_enabled`,
  `global_hotkey`. Caret anchoring and inline filtering have no stored switches.
- `echo-windows/shell/hotkey.rs`: RegisterHotKey/UnregisterHotKey on the existing
  tray thread; MOD_NOREPEAT; no low-level keyboard hook or new resident process.
- `echo-windows/focus.rs`: foreground process/start-time/control snapshot at the
  WM_HOTKEY boundary, before Echo takes focus. Native GUI caret first.
- `echo-windows/focus/automation.rs`: one no-window MTA worker for UIA and MSAA.
  UIA TextPattern2 caret, degenerate TextPattern selection, approximate adjacent
  character on a cloned range, MSAA caret, input-control bounds. No actual user
  selection is changed. Password/read-only/unverified targets cannot auto-paste.
- `echo-windows/focus/placement.rs`: physical-pixel work-area and front-card
  placement, transparent stage offset, up/down flip, bounded compact height.
- `desktop/service/capture_lane.rs`: a bounded, storage-independent capture thread.
  The UI accepts a current epoch and queues adoption before any Execute.
- `desktop/service.rs`: priority queue for Adopt/Cancel/Execute; retained target
  state changes only on the domain worker. No recapture at show.
- `desktop/app/quick_insert_window.rs`: size and position before show, compact
  Cover Flow stage with unchanged front-card renderer, separate manager geometry.

The MTA caller waits at most 200 ms, including embedded MSAA focus resolution.
A timeout does **not** cancel the external COM
call: the singleton worker remains busy until that call finishes, and no new
worker is spawned. Late results cannot upgrade an already shown copy-only
session. The next invocation can retry when the worker is available. No timer
tracks another application's caret while Echo is hidden or visible.

Placement and insertion authorization are separate. A usable control/window
fallback can position the popup without authorizing a paste. In that case the
UI offers Copy and reports that no safe target was captured. A valid target is
revalidated at delivery; changing the original focused control or destroying its
window does not redirect the payload to the current foreground window.

Native coordinates are converted in the source window's DPI context. UIA/MSAA
rectangles already represent physical screen coordinates. The anchor monitor's
work area and DPI drive physical placement; the Slint content stays logically
scaled. The desired 900-dip Quick Insert stage retains a 520-dip front card on a
normal display, rather than anchoring the corner of a 1600-dip transparent stage.
At horizontal edges the visible card is positioned first, then excess symmetrical
transparent stage space is removed. The actual text-card width is preserved.
A clean Quick Insert dismisses on external focus loss. Dirty dialogs retain their
draft, but invalidate the old paste target; internal Echo menus do not dismiss.

## Transactional hotkey updates

Settings saves reserve the new combination first, keeping the old registration.
Only after the storage revision/transaction succeeds does the tray thread commit
the switch and release the old registration. Failed registration or persistence
leaves the previous binding intact. Reservation drop aborts an uncommitted
candidate without blocking Drop. Requests claimed by the tray thread return their
actual result, not a guessed timeout outcome. After a native commit failure,
`service/settings_commit.rs` rolls back only persisted shortcut fields; an
optimistic-revision conflict reloads current settings and reports the runtime
mismatch explicitly. SQLite and Win32 are not described as one atomic transaction.
Queued WM_HOTKEY messages are checked against both the active ID and
combination; old/candidate registrations are not activation requests.

An isolated development/test instance can use `ECHO_DISABLE_GLOBAL_HOTKEY=1`.
This does not overwrite the stored enabled flag and has a distinct runtime
status. Do not run a production resident and a hotkey acceptance instance with
the same binding at the same time unless testing the conflict path.

## Paste safety

`ClipboardPlatform::paste_preflight` is specific to Insert; explicit Copy does
not run it. Windows checks target identity/integrity and currently held Alt,
Ctrl, Shift and Windows keys before clipboard staging. A held modifier rejects
insertion; Echo never forces the user's physical modifier keys up. Delivery
checks the control and foreground again. Native WM_PASTE uses a bounded
SendMessageTimeout; UIA-backed input uses SendInput only after verification.
Native timeouts can be ambiguous if the other app is already processing a paste;
check the target before retrying rather than assuming a timed-out app did nothing.

Quick Insert also accepts a focused UIA Group when its TextPattern explicitly
reports a writable document range (for example, the Feishu chat composer).
Enabled, non-password, window ownership and runtime identity checks still apply
at capture and delivery. Inline support additionally requires an editor-scoped
selectable range and the normal replacement/composition checks.
If the initial inline inspection fails, desktop retries ordinary Quick Insert
capture using the still-current pre-show snapshot before showing the popup.
Expired snapshots and failures of an already active completion do not authorize
a new paste target. The fallback notice reflects whether ordinary insertion is
available or only manual copying is possible.

## Validation

`tests/native/Invoke-GroupQuickInsertAcceptance.ps1 -EvidenceRoot <new-directory>`
requires `ECHO_WINDOWS_ACCEPTANCE=1` and runs a real TextPattern-only WPF Group
fixture through production capture and paste. It preserves the clipboard and
uses isolated synthetic data. The regression covers Group/Edit/Document,
read-only and missing-pattern rejection, focus loss, changed identity and a
destroyed target. Real Feishu compatibility must be validated separately.
The same fixture exercises inline query observation, backspace, navigation,
Enter replacement with preserved surrounding text, no-result Enter protection,
out-of-query selection rejection and Escape cancellation. Its injected input
does not establish physical Chinese IME acceptance or desktop popup behavior.

Pure tests cover shortcut syntax/defaults/round trips, key mapping/stale hotkey
messages, invalid accessibility rectangles, negative-coordinate monitors,
96/144/192 DPI, edge flips and front-card offsets. They do not replace physical
mixed-DPI or application compatibility testing.

`tests/native/Invoke-CaretHotkeyAcceptance.ps1` and `CaretHotkeyScenarios.ps1`
exercise real RegisterHotKey and owned native/WPF input windows using a copied,
capture-disabled synthetic database. They check runtime rebind/conflict/disable,
restart persistence, independent caret/card bounds, native and UIA insertion,
copy-only controls, held modifiers, changed controls and destroyed windows.
`EchoWpfCaretFixture.cs` provides a UIA-only input without an Edit HWND.
Screenshots require an explicit `native-test` binary and capture only Echo's
Slint surface, not the desktop. The distributable binary must be rebuilt without
that feature. Gate summaries must distinguish PASS/FAIL/NOT_RUN; physical Chinese
IME, apps not actually exercised, and physical mixed-DPI/soak are not implied by
fixture success.
