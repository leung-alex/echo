# Test and Acceptance Strategy

## Objective

Avoid redundant full regressions while maintaining confidence.

Use three levels:

```text
Local tests
    ↓
Contract/integration smoke
    ↓
ONE final full acceptance
```

## Level 1 — Repository-local tests

Run continuously inside the owning repository.

### Culsans

- command
- shell settings
- input
- browser integration
- external app discovery
- activation encoding/launch
- unavailable external app behavior

### Aster

- search
- filters/scope
- Everything lifecycle
- UI
- diagnostics
- settings
- activation

### Iris

- capture
- capture editor
- pin
- drawing
- persistence
- settings
- activation

### Echo

- listener
- storage
- blobs
- History/Favorites/Snippets
- Quick Insert
- paste target
- settings
- activation

Do not run unrelated repositories' suites for a local change.

## Level 2 — Integration Smoke Gate

Run when a major stream reaches “externally launchable”.

Test only:

1. Culsans finds app.
2. Culsans launches app.
3. activation payload is accepted.
4. repeated activation uses intended single-instance behavior.
5. app close/reopen works.
6. missing app is handled.
7. external app crash does not kill Culsans.
8. Culsans close does not corrupt external app state.

Do not run full capture/search/clipboard regression here.

## Level 3 — Final Full Acceptance

Run once after cutover.

### Shell

- Command Panel startup latency
- global shortcut behavior
- input/gesture regression
- browser integration
- settings
- tray/lifecycle
- window/focus safety
- app unavailable state

### Aster

- open from Culsans
- direct launch
- query
- filters
- scope
- result open
- Everything prewarm
- Everything restart/failure
- settings
- clean shutdown

### Iris

- capture
- cancel
- precision capture
- editor
- copy/save
- pin
- drawing CRUD
- drawing library
- persistence
- repeated activation

### Echo

- clipboard ingestion
- History
- Favorites
- Snippets
- Quick Insert
- copy
- paste to original target
- invalid/closed target
- image/rich content
- retention/GC
- background start/stop
- repeated activation

### Data migration

- clean install
- upgrade from legacy Culsans data
- idempotent rerun
- backup created
- missing blob behavior
- drawing preservation

### Process/lifecycle

- Culsans starts with apps missing
- apps start without Culsans
- independent product shutdown
- product crash isolation
- Windows logon/background startup where configured
- uninstall one product without breaking others

### Packaging

- Culsans installer does not carry Everything
- Aster installer owns Everything resources
- Culsans frontend output no longer includes Iris-only heavy dependencies
- independent versioning works

## Performance baseline

Measure before and after with the same machine/profile.

Record:

- Culsans cold start to Command Panel interactive
- Culsans idle private working set
- Culsans process count
- Aster first activation latency
- Aster warm activation latency
- Iris capture activation latency
- Echo Quick Insert activation latency
- Echo background memory
- total idle footprint with only intended resident processes

Do not claim a performance win based only on repository separation.

## Architecture acceptance grep/checks

The final reviewer should assert:

- no sibling `path = "../..."` Cargo dependency
- no sibling `file:../...` npm dependency
- no Culsans dependency on `culsans-search` for File Search
- no Culsans dependency on `culsans-clipboard`
- no Culsans dependency on `culsans-capture`
- no Culsans drawing persistence
- no Everything bundled by Culsans
- no Excalidraw/Fabric in Culsans unless separately justified
- no Culsans writes to Echo/Iris data
