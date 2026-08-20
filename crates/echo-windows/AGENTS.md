# Windows Adapter Ownership

This crate owns Windows clipboard, focus, target capture, and paste delivery.
Read `docs/architecture/overview.md` and
`docs/architecture/dependency-rules.md`. Keep unsafe Win32 code local and do
not depend on SQLite, storage, Tauri, or frontend code.
