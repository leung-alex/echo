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

## Transport

Commands carry requests and responses, events carry small invalidation signals,
and preview bytes have a dedicated future binary seam. R0 retains the existing
preview command behavior; the preview module does not introduce a new Base64
alternative. Checked-in TypeScript transport types are verified by the tooling
drift check.

The removed legacy Snippets concept has no engine module, UI route, command,
transport type, or SQLite table. Saved Items are the only durable reusable-item
concept.
