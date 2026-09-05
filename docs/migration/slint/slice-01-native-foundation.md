# Slice 01 — native foundations

The working branch is codex/ui. Main is not modified.

## Implemented

- echo-presentation: framework-independent query epochs, page generations,
  source+i64 keys, bounded 500-row windows with backward cursors, stable
  selection, batch selection, keyboard/IME policy and insertion session epochs.
- echo-windows::shell: user/session/data-root isolated single instance;
  user-only local named-pipe ACL, explicit peer SID validation, bounded messages,
  overlapped I/O deadlines and cancellation; no TCP server.
- Event-driven native tray with Explorer restart support and explicit Quit.
- Native owner, Mica/fallback, DPI-aware positioning, resize hit testing,
  input-method state and main-window geometry notifications.
- Existing clipboard, storage and engine behavior is unchanged.

## Actually executed on the authorized Windows computer

`cargo test -p echo-windows -p echo-presentation`: PASS (27 presentation,
9 Windows adapter tests). This is not yet an integrated Slint acceptance.
Original `echo.cmd verify`: PASS including 21 browser acceptance/visual tests,
Rust tests and canonical storage cleanup gate before the product replacement.
Original Release was built with rustc 1.97.0 and archived outside Git.
30 independent original Release launches were measured via CDP semantic
readiness; the raw metric explicitly includes CDP observation overhead.
Actual original History/Favorites screenshots were saved outside Git.

Evidence root: D:/Worktrees/echo/migration-evidence/20260905-native
Original source SHA: 0dc699e42d8d667e502938d71e33216f92513e5a

The earlier G0 blocker was a missing execution interface, now resolved.
The user reauthorized completing the native migration. Outstanding measurements
remain outstanding rather than being labelled PASS. The immutable original
binary and source allow paired measurements throughout the integration.