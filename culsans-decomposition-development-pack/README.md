# Culsans Decomposition Development Pack

**Target:** split the current Culsans repository into four fully independent sibling projects:

```text
D:\Projects\culsans
D:\Projects\aster
D:\Projects\iris
D:\Projects\echo
```

Each directory is an independent Git repository, build, release, installer, version, data owner, and runtime owner.

This package is intended to be handed directly to multiple Codex sessions.

---

## Locked product boundaries

| Project | Product role | Owns |
|---|---|---|
| `culsans` | Command Shell / Control Plane | Command Panel, application registry, semantic action routing, Input Agent bridge, gesture/input, browser integration, shell settings, window/focus context, shell presentation |
| `aster` | Find | File Search UI, file-search runtime, Everything integration/lifecycle, filters/scopes, diagnostics, search settings |
| `iris` | See | Capture, precision capture, capture editor, pin, annotation, drawing, drawing library, Fabric/Excalidraw dependencies, visual settings |
| `echo` | Recall | Clipboard listener, history, favorites, snippets, Quick Insert, insertion/paste workflow, clipboard persistence/blobs/GC, Echo settings |

---

## Non-negotiable independence rules

1. **No common parent Git repository.**
2. **No Cargo workspace spanning sibling directories.**
3. **No pnpm workspace spanning sibling directories.**
4. **No `path = "../culsans/..."` dependencies.**
5. **No `file:../culsans/...` npm dependencies.**
6. **No Git submodules linking the products.**
7. **No shared SQLite database.**
8. **No shared mutable data directory.**
9. **No shared runtime object.**
10. **No source-level dependency from Aster/Iris/Echo back to Culsans.**
11. Cross-app interaction must occur only through an explicit external activation contract.
12. Each product must be independently buildable and runnable with the other three source trees absent.

---

## Current-state evidence to remove

The current `culsans` Cargo workspace contains, among others:

- `backend/crates/culsans-clipboard`
- `backend/crates/culsans-capture`
- `backend/crates/culsans-runtime`
- `backend/crates/culsans-search`
- `backend/crates/culsans-storage`

The desktop host directly depends on capture, runtime, search and storage.

The frontend application currently includes heavy visual dependencies such as:

- `@excalidraw/excalidraw`
- `fabric`
- `html-to-image`

The current Culsans Tauri bundle also carries `Everything.exe`.

These are explicit decomposition targets.

---

## Execution model

This is **one architecture program with one final cutover**, not three sequential product extractions.

```text
                         TARGET CONTRACT
                               │
      ┌────────────────────────┼────────────────────────┐
      │                        │                        │
      ▼                        ▼                        ▼
    ASTER                    IRIS                     ECHO
  extraction              extraction              extraction
      │                        │                        │
      └──────────────┬─────────┴─────────┬──────────────┘
                     │                   │
                     ▼                   ▼
               CULSANS SHELL       DATA MIGRATION
               RUNTIME HOLLOWING
                     │
                     ▼
                INTEGRATION GATE
                     │
                     ▼
                  CUTOVER
                     │
                     ▼
             ONE FULL ACCEPTANCE
```

Four implementation streams run concurrently:

- Stream A — Aster
- Stream B — Iris
- Stream C — Echo
- Stream D — Culsans shell/runtime contraction

A fifth Codex session should act as integration/review authority and avoid feature development unless an integration defect is assigned to it.

---

## Recommended reading order for Codex

1. `docs/01_TARGET_ARCHITECTURE.md`
2. `docs/02_PARALLEL_EXECUTION.md`
3. `docs/03_EXTERNAL_APP_PROTOCOL_V1.md`
4. The relevant repository plan:
   - `docs/04_CULSANS_PLAN.md`
   - `docs/05_ASTER_PLAN.md`
   - `docs/06_IRIS_PLAN.md`
   - `docs/07_ECHO_PLAN.md`
5. `docs/08_DATA_MIGRATION.md`
6. `docs/09_TEST_AND_ACCEPTANCE.md`
7. `docs/10_CUTOVER_AND_ROLLBACK.md`
8. The matching prompt under `prompts/`
9. The matching tickets under `tickets/`

---

## Definition of Done

The decomposition is complete only when all of the following are true:

- `D:\Projects\aster` builds and runs without the Culsans source tree.
- `D:\Projects\iris` builds and runs without the Culsans source tree.
- `D:\Projects\echo` builds and runs without the Culsans source tree.
- Culsans builds and runs with **none** of the three product implementations in its source tree.
- Culsans no longer bundles Everything.
- Culsans frontend no longer carries Iris-only visual dependencies.
- Culsans runtime no longer owns clipboard, file-search, drawing, or capture business state.
- Echo owns its clipboard database/blob directory.
- Iris owns its drawing persistence.
- Aster owns its search backend/resources/settings.
- Cross-app launching works through the external activation contract.
- Local tests pass in each repository.
- One final cross-product acceptance run passes after cutover.
