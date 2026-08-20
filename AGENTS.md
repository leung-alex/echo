# Echo Architecture Guide

Echo is a standalone Windows clipboard history and Quick Insert application.
Use the repository map and ownership rules below before changing code.

## Source Of Truth

- Architecture: `docs/architecture/overview.md`
- Dependency direction: `docs/architecture/dependency-rules.md`
- Domain language: `docs/domain/glossary.md`
- Change locality: `docs/architecture/locality.md`
- Test ownership: `docs/TEST_OWNERSHIP_MAP.md`
- Workflow and gates: `docs/engineering/agent-workflow.md`

## Repository Map

- `crates/echo-engine`: domain behavior, ingestion, History, Saved Items,
  Quick Insert, settings, and adapter interfaces.
- `crates/echo-storage`: SQLite/blob/search/migration adapter.
- `crates/echo-windows`: Windows clipboard, focus, and paste adapter.
- `crates/echo-activation`: small Echo activation envelope and flag protocol.
- `apps/desktop`: Tauri composition root, commands, events, and transport DTOs.
- `apps/ui`: React presentation and feature-owned IPC clients.
- `tools/echo`: canonical developer gates and changed-owner planning.

## Where To Change X

- Domain policy or use case: `crates/echo-engine`.
- SQLite, blob, search, or migration behavior: `crates/echo-storage`.
- Win32 clipboard/focus/paste behavior: `crates/echo-windows`.
- Activation parsing or naming: `crates/echo-activation`.
- Tauri wiring or DTO mapping: `apps/desktop`.
- UI behavior: the owning feature under `apps/ui/src/features`.
- Raw Tauri IPC: only `apps/ui/src/shared/ipc`.

## Fixed Worktrees

| Branch | Path |
| --- | --- |
| `main` | `D:\Project\echo` |
| `codex/foundation` | `D:\Worktrees\echo\foundation` |
| `codex/ui` | `D:\Worktrees\echo\ui` |

Run `echo.cmd sync` interactively, or use `--all` / `--branch` in scripts, to
fast-forward clean fixed branches to the current local `main`. Unsafe branches
with local changes, ahead commits, or divergence are never modified.

## Hard Rules

- Dependencies point from adapters and desktop toward engine; engine never
  imports Tauri, SQLite, Win32, or frontend types.
- Keep transport DTOs separate from engine domain types.
- Durable reusable content is represented only by Saved Items; do not recreate
  a removed reusable-content surface or compatibility layer.
- The canonical activation flag is `--echo-activate`.
- Unsafe Win32 code stays in `crates/echo-windows`.

## Validation

Run `.\echo.cmd self-check`, `.\echo.cmd format --check`,
`.\echo.cmd bindings --check`, and `.\echo.cmd verify` for the canonical
developer gates. Native acceptance is opt-in and must use the authorized
`ECHO_WINDOWS_ACCEPTANCE=1` gates.
