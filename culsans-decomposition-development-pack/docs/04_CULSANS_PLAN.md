# Culsans Shell / Runtime Contraction Plan

## Mission

Reduce Culsans to a focused command shell/control plane while keeping:

- global input
- command dispatch
- browser integration
- shell settings
- app discovery/launch
- window/focus context
- shell presentation

## Current coupling targets

The current workspace directly includes:

- `culsans-clipboard`
- `culsans-capture`
- `culsans-runtime`
- `culsans-search`
- `culsans-storage`

The desktop host directly depends on capture, search, runtime, and storage.

The current frontend app carries Iris-only dependencies such as Excalidraw and Fabric.

The current Tauri bundle carries Everything.

All of these facts must change.

## Target Culsans runtime

Conceptual target:

```text
ShellRuntime
├── command
├── app_registry
├── action_router
├── input
├── browser
├── shell_settings
├── window_context
└── shell_presentation
```

Must not contain:

```text
clipboard_service
quick_insert_service
file_search_service
drawing_store
capture_service
everything_process
snippet_repository
```

## Work packages

### C1 — External App Registry

Implement:

- application identity: Aster/Iris/Echo
- executable discovery
- developer path override
- installed/unavailable state
- launch errors with user-visible diagnostics

No business APIs.

### C2 — Semantic Action Router

Map Culsans commands to external activation actions.

Examples:

```text
Search Files -> aster.search
Capture -> iris.capture
Precision Capture -> iris.capture_precision
Drawing -> iris.drawing
Quick Insert -> echo.quick_insert
```

Do not expose internal app state in Culsans.

### C3 — File Search hollowing

After Aster local verification:

- remove File Search runtime ownership
- remove Search implementation dependency from desktop
- remove File Search frontend route/components owned by Aster
- remove Everything resources/config from Culsans installer
- remove File Search settings from Culsans settings ownership
- keep only semantic command entries

### C4 — Iris hollowing

After Iris local verification:

- remove capture/drawing business commands from Culsans
- remove capture/drawing persistence ownership
- remove visual frontend routes/components
- remove Excalidraw/Fabric/html-to-image if no shell feature still needs them
- remove visual settings from Culsans

### C5 — Echo hollowing

After Echo local verification:

- remove clipboard listener ownership
- remove clipboard persistence/business models
- remove History/Favorites/Snippets/Quick Insert UI
- remove Echo-specific settings
- remove blob maintenance from Culsans
- retain only semantic activation commands

### C6 — Storage split

Refactor `culsans-storage` to contain only Culsans-owned data.

Any table/API whose actor is Aster/Iris/Echo must leave.

Do not retain deprecated APIs “just in case”.

### C7 — Contract cleanup

Culsans TypeScript/Rust contracts should contain only:

- shell DTOs
- shell settings
- input/browser/window/shell diagnostics
- external app availability/launch result

Delete external-product internal DTOs from Culsans.

### C8 — Packaging cleanup

Culsans installer must not package:

- Everything
- Iris-specific visual assets/dependencies
- Echo-specific data/bootstrap code

Culsans description should be updated to describe the Command Shell rather than the old all-in-one product.

## Local acceptance

Culsans passes local acceptance when:

- it starts without Aster/Iris/Echo installed,
- unavailable apps produce clean disabled/error behavior,
- Command Panel still opens on the hot path,
- Input Agent behavior is unaffected,
- browser integration is unaffected,
- shell settings work,
- installing an external app makes its actions launchable,
- external app crash does not crash Culsans,
- no sibling source directory is required to build.
