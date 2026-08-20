# ADR-001: Contract Hardening

## Decision

Keep domain behavior in `echo-engine` and expose only its domain contracts.
`echo-storage`, `echo-windows`, and `echo-activation` remain independent
adapters. `apps/desktop` composes them and owns Tauri transport DTOs. The
checked-in TypeScript file is generated from that Rust transport source; UI
features use the shared IPC wrapper.

Storage uses a dedicated writer runtime and independent reader connection. The
writer owns ordered migrations and debounced maintenance; normal capture does
not reconcile the whole store. FTS5 MATCH queries and stable cursors remain a
storage concern. Preview resources are binary-safe content-addressed files,
not an encoded image transport.

## Consequences

New behavior starts in the primary module named by
`docs/architecture/locality.md`. Adapter code translates the domain contract
without copying policy. Verification rejects forbidden dependency edges, raw
frontend invokes, stale generated bindings, removed product vocabulary,
polling, and per-capture reconciliation.
