# UI Ownership

Presentation belongs here under `src/features/history`,
`src/features/quick-insert`, `src/features/saved-items`, and
`src/features/settings`. Read `docs/architecture/overview.md`. Feature code
uses typed clients under `src/shared/ipc`; it does not call raw Tauri IPC.
History refreshes from initial/query/view/event/pagination responses, not a
background polling loop. Preview images load through the bounded visible-row
path; do not add a process-wide image cache.
