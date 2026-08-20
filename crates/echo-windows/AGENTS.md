# Windows Adapter Ownership

This crate owns Windows clipboard, focus, target capture, and paste delivery.
Read `docs/architecture/overview.md` and
`docs/architecture/dependency-rules.md`. Keep unsafe Win32 code local and do
not depend on SQLite, storage, Tauri, or frontend code.

This is the single native adapter for Quick Insert paste delivery. Keep target
capture, focus validation, and delivery here rather than in desktop or UI.
