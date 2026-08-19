# Echo Extraction Plan

## Product statement

**Echo — Recall**

A standalone reusable-content and fast-insertion application.

Clipboard is the ingestion engine; Quick Insert is the primary retrieval/insertion surface.

## Domain model

```text
Echo
├── Clipboard Engine
│   ├── listener
│   ├── ingestion
│   ├── representations
│   ├── blobs
│   └── maintenance
├── Library
│   ├── History
│   ├── Favorites
│   └── Snippets
└── Quick Insert
    ├── search/browse
    ├── copy
    └── insert/paste
```

Favorites are not a peer application. Quick Insert is not a fourth content domain.

## Source ownership to migrate

Inventory and migrate:

- `culsans-clipboard`
- clipboard listener lifecycle
- clipboard normalization/sanitization
- representations
- clipboard DB tables
- blob files and GC
- History
- Favorites / saved items
- Snippets
- Quick Insert domain
- Quick Insert UI
- insertion service
- paste target validation
- clipboard settings
- Echo-specific tests

## Repository bootstrap

`D:\Projects\echo`

Recommended shape:

```text
echo\
├── apps\
│   ├── desktop\
│   └── agent\        # optional if separated
├── backend\
│   └── crates\
│       ├── echo-clipboard\
│       ├── echo-storage\
│       └── echo-platform-windows\
├── frontend\app\
└── tests\
```

## Runtime model

The clipboard listener must not require the Quick Insert WebView to stay open.

Preferred steady state:

```text
lightweight Echo background host/agent
        │
        ├── clipboard listener
        ├── persistence
        └── maintenance

Echo UI
        └── created/shown on demand
```

An initial implementation may package both in one product, but ownership must remain Echo-only.

## Extraction strategy

### E1 — Clipboard background ownership

Move listener/service lifecycle first.

Verify that Culsans can run without listening to the clipboard.

### E2 — Persistence ownership

Echo owns:

```text
%LOCALAPPDATA%\Echo\
    echo.sqlite3
    blobs\
```

The current clipboard schema and saved insert/snippet data migrate here.

### E3 — Library

Preserve:

- History
- Favorites
- Snippets

with their current behavior.

### E4 — Quick Insert

Move the complete UI and action workflow.

### E5 — Paste target

Echo owns the actual paste behavior and target validation.

Culsans may provide optional origin window context in the activation envelope, but Echo validates and uses it.

### E6 — Settings

Echo owns:

- history enabled
- retention
- max entries/bytes
- privacy/window-title behavior
- draft recovery if still applicable
- paste behavior
- snippet/library settings

### E7 — Activation

Implement:

- `echo.open`
- `echo.quick_insert`
- `echo.settings`

### E8 — Packaging

Echo has its own:

- identifier
- installer
- version
- data path
- startup/background policy

## Data migration

Current clipboard data uses `culsans.sqlite3` and a `blobs` directory.

Migration must preserve:

- clipboard entries
- representations
- pinned/favorite state
- saved insert items
- snippets
- settings
- referenced blob content

Migration must be idempotent and backup-first.

## Local tests

Required:

- clipboard listener
- duplicate/fingerprint behavior
- representations
- blob storage
- GC/reconciliation
- History
- Favorites
- Snippets
- copy
- insert/paste
- invalid target handling
- activation when running/not running
- history disabled
- retention
- independent build without Culsans

## Exit condition

Echo is complete only when Culsans no longer opens or writes clipboard business data and no longer owns the clipboard listener/Quick Insert workflow.
