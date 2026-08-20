# UI Ownership

Presentation belongs here under `src/features/history`,
`src/features/quick-insert`, `src/features/saved-items`, and
`src/features/settings`. Read `docs/architecture/overview.md`. Feature code
uses typed clients under `src/shared/ipc`; it does not call raw Tauri IPC.
