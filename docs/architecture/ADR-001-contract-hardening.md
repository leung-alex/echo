> Historical ADR. The domain/storage decisions remain relevant, but its Tauri transport and generated TypeScript decisions were superseded by the native Slint migration. Current contracts are documented in `docs/architecture/overview.md` and `docs/architecture/dependency-rules.md`.

# ADR-001: Contract Hardening

## Historical Decision

Keep domain behavior in `echo-engine` and expose only its domain contracts. `echo-storage`, `echo-windows`, and `echo-activation` remain independent adapters. At the time of this ADR, `apps/desktop` composed them through Tauri transport DTOs, a checked-in TypeScript file was generated from Rust transport source, and browser UI features used a shared IPC wrapper.

Storage uses a dedicated writer runtime and independent reader connection. The writer owns ordered migrations and debounced maintenance; normal capture does not reconcile the whole store. FTS5 MATCH queries and stable cursors remain a storage concern. Preview resources are binary-safe content-addressed files, not an encoded image transport.

## Current Status

The storage, domain-boundary, FTS5, cursor, preview, and maintenance decisions remain active. The Tauri, TypeScript, browser IPC, and generated frontend-binding decisions are historical and no longer required.

The native replacement uses `echo-presentation` for framework-independent presentation contracts, typed Rust worker/event messages in `apps/desktop`, compiled Slint components under `apps/desktop/ui`, and native shell services in `echo-windows`.

## Consequences

New behavior starts in the primary module named by `docs/architecture/locality.md`. Adapter code translates the domain contract without copying policy. Active verification rejects forbidden dependency edges, browser-stack remnants, stale row-key parsing, polling, and per-capture reconciliation while preserving the canonical storage leak gate.
