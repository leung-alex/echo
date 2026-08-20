# Echo Architecture Overview

## Direction

Echo has one business module and explicit adapters:

```text
echo-activation                 (independent protocol)
        ^
apps/desktop  -> echo-engine <- echo-storage
      |                ^
      +-> echo-windows-+
      |
   apps/ui (desktop transport only)
```

`echo-engine` owns behavior and declares the interfaces it needs. Storage and
Windows implement those interfaces. Desktop composes the objects and maps
domain values to transport DTOs; it does not implement business policy.

## Ownership

### Engine

`ingest` normalizes clipboard snapshots and applies capture policy. `history`
owns history queries and actions. `saved_items` owns the durable favorite
concept. `quick_insert` owns retrieval, target session, copy, and insert
orchestration. `settings` owns settings values. `preview` owns preview asset
requirements without transport encoding.

### Adapters

`echo-storage` owns SQLite schema, FTS, blob files, migrations, and persistence
mapping. `echo-windows` owns clipboard listeners, format access, target capture,
focus validation, clipboard writes, and paste delivery. Unsafe Win32 code is
local to that crate.

### Shell and presentation

`apps/desktop` is the Tauri composition root with feature commands, events,
activation, and transport DTOs. `apps/ui` owns presentation in separate
`history`, `quick-insert`, `saved-items`, and `settings` areas. Raw invokes are
wrapped by `apps/ui/src/shared/ipc`.

## Runtime Flow

```text
Windows clipboard event
  -> echo-windows ClipboardSource
  -> echo-engine ingest
  -> echo-storage persistence
  -> desktop history invalidation/commands
  -> apps/ui presentation
```

Quick Insert activation captures the paste target before Echo is shown. The UI
must not capture it again after the native activation handoff.

Storage runtime ownership, ordered schema migrations, instrumentation, and the
deterministic storage diagnostic are documented in
`docs/architecture/storage-runtime.md`.

## Transport

Commands carry requests and responses, events carry small invalidation signals,
and preview bytes use a dedicated binary resource seam. Checked-in TypeScript
transport types are generated from the desktop transport and verified by the
tooling drift check. History and Saved Item search use bounded FTS5 MATCH
queries over stable cursors; no background polling is required.

Saved Items are the only durable reusable-item concept. A History save creates
a linked snapshot; a user edit marks that
snapshot independent, so unsaving History removes only an untouched snapshot
and unlinks an edited item without destroying its content. Saved Item deletion
is always explicit.
