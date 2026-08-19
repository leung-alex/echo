# Aster Extraction Plan

## Product statement

**Aster — Find**

A standalone Windows search application that owns file search end to end.

## Source ownership to migrate

From Culsans, inventory and migrate all behavior related to:

- file-search UI
- file-search IPC/Tauri commands
- search service
- `culsans-search` capabilities required by File Search
- Everything integration
- Everything process lifecycle
- Everything configuration
- Everything bundled binary/resources
- prewarm/index behavior
- filters
- search scope
- result actions
- diagnostics
- search settings
- File Search tests

Do not blindly copy unrelated command-search functionality that only belongs to Culsans Command Panel.

## Repository bootstrap

`D:\Projects\aster`

Recommended internal shape:

```text
aster\
├── apps\
│   └── desktop\
├── backend\
│   └── crates\
│       ├── aster-search\
│       ├── aster-platform-windows\   # only if justified
│       └── ...
├── frontend\
│   └── app\
├── tests\
├── Cargo.toml
├── package.json
└── README.md
```

This is illustrative, not mandatory. Locality is more important than matching old Culsans layout.

## Extraction strategy

### A1 — Build a standalone vertical slice first

Get this path working inside Aster:

```text
Aster window
→ query input
→ search command
→ search backend
→ results
→ open result
```

Use copied/migrated code as necessary, but eliminate Culsans source dependencies immediately.

### A2 — Move Everything ownership

Aster must own:

- executable/resources
- configuration
- launch/shutdown policy
- diagnostics
- prewarm/background policy
- failure/reconnect behavior

Culsans must not manage Everything after cutover.

### A3 — Move UI

Migrate File Search UI as Aster's primary interface.

Preserve behavior before redesigning.

### A4 — Move settings

Aster settings include only search concerns:

- prewarm
- backend/index policy
- filters/default scope
- result behavior
- diagnostics

Culsans only offers “Open Aster Settings”.

### A5 — Single-instance activation

Implement v1 actions:

- `aster.open`
- `aster.search`
- `aster.settings`

If Aster is running, activation goes to the existing instance.

### A6 — Independent packaging

Aster must have:

- its own Tauri identifier
- own icon/product metadata
- own installer
- own version
- own resources
- own data directory

## Data ownership

Aster should not use `culsans.sqlite3`.

If File Search currently has no durable user data beyond settings/backend state, start with a minimal Aster data directory.

If future tags/catalog state are introduced, they belong to Aster.

## Local tests

Required:

- query/result behavior
- filter/scope behavior
- Everything unavailable
- Everything startup/reconnect
- prewarm setting
- result open action
- activation while not running
- activation while already running
- settings persistence
- independent clean-machine build

## Exit condition

Aster is complete only when Culsans can delete every File Search implementation dependency and still launch Aster by semantic action.
