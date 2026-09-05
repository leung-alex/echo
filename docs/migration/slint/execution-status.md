# Native migration execution status

Target branch: `codex/ui` (the remote UI branch).
Original product source: `0dc699e42d8d667e502938d71e33216f92513e5a`.

This change is M0 measurement preparation, not the Slint migration.
G0 is NOT RUN: no authorized interactive Windows execution connection was available during implementation.
No production Rust, React, Tauri, storage, clipboard behavior or dependency versions have been changed.

Run `go -C tools/echo run ./perf-native help` for the preparation commands.
Tool unit tests use synthetic data; their success is not Windows acceptance evidence.
Do not proceed with M1 or remove the old runtime until the original Release baseline has been measured and reviewed.
