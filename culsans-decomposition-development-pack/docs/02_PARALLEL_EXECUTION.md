# Parallel Execution Plan

## Goal

Perform Aster extraction, Iris extraction, Echo extraction, and Culsans runtime contraction **concurrently**, then execute one cutover and one full regression.

## Agent topology

Use five Codex sessions:

| Agent | Workspace | Role |
|---|---|---|
| A | `D:\Projects\aster` + read access to `D:\Projects\culsans` | Aster extraction |
| B | `D:\Projects\iris` + read access to `D:\Projects\culsans` | Iris extraction |
| C | `D:\Projects\echo` + read access to `D:\Projects\culsans` | Echo extraction |
| D | `D:\Projects\culsans` | Shell/runtime contraction and external-app launcher |
| E | Read access to all four | Integration reviewer / acceptance authority |

### Write ownership

- Agent A writes only `aster`, except when explicitly assigned a tiny source-removal patch in Culsans.
- Agent B writes only `iris`, except when explicitly assigned a tiny source-removal patch in Culsans.
- Agent C writes only `echo`, except when explicitly assigned a tiny source-removal patch in Culsans.
- Agent D owns all structural removals and contract integration in `culsans`.
- Agent E should default to read-only review. Integration fixes are assigned to the owning agent whenever possible.

This avoids recreating the current conflict hotspot in `culsans-runtime`, desktop composition, contracts, and frontend routing.

---

## Phase P0 — Baseline and freeze

**Duration model:** short, mandatory, no feature work.

All agents record:

- current main commit SHA
- current build/test commands
- current relevant user flows
- current data paths
- current Windows process behavior
- current file-search/capture/clipboard settings behavior

The baseline must remain available as the migration reference.

No one should opportunistically redesign features during extraction.

### Freeze scope

Freeze behavior for:

- File Search
- Capture / Pin / Drawing
- Clipboard / Favorites / Snippets / Quick Insert

Only decomposition defects may alter behavior.

---

## Phase P1 — Independent repository bootstrap

A, B, C happen in parallel.

Each new repo gets:

- its own `.git`
- README
- Cargo workspace if Rust requires multiple local crates
- pnpm workspace only inside that repo if needed
- formatter/lint config
- build/test scripts
- Tauri application shell where applicable
- product identifier
- independent data directory
- independent installer config
- no cross-sibling path dependencies

Agent D simultaneously introduces the external application registry/launcher seam into Culsans **without** removing old implementations yet.

This is the only short expand step.

---

## Phase P2 — Parallel ownership migration

Run four streams simultaneously.

### Stream A — Aster

Migrate the complete File Search vertical slice:

```text
UI
→ contracts internal to Aster
→ Tauri commands
→ search service
→ Everything lifecycle/resources
→ settings
→ diagnostics
→ tests
```

### Stream B — Iris

Migrate the complete visual workflow:

```text
Capture
→ editor
→ annotation
→ pin
→ drawing
→ drawing library
→ visual persistence
→ settings
→ tests
```

### Stream C — Echo

Migrate the complete recall/insertion workflow:

```text
clipboard listener
→ persistence
→ history/favorites/snippets
→ Quick Insert
→ copy/insert
→ paste target/focus safety
→ maintenance
→ settings
→ tests
```

### Stream D — Culsans hollowing

As each external app reaches a verified local milestone, remove corresponding ownership from Culsans on an integration branch:

- externalize commands
- remove runtime state/service fields
- remove business storage access
- remove frontend routes/components
- remove Tauri commands
- remove settings pages for external products
- remove package/crate dependencies
- remove bundled third-party resources

Do **not** perform a full system regression after each removal.

---

## Phase P3 — Integration Gate

This is a lightweight cross-product gate, not full acceptance.

Verify:

1. Culsans discovers all three apps.
2. Culsans launches each app.
3. Repeated activation does not create broken duplicate instances.
4. The semantic action reaches the intended app.
5. App close/reopen works.
6. Culsans survives external app crashes.
7. Each app survives Culsans closing after it has launched.
8. No sibling source dependency exists.
9. Data ownership points to the new product directories.

Fix only integration blockers.

---

## Phase P4 — Single Cutover

After all four streams are ready:

1. Back up legacy Culsans data.
2. Run one-time data migration.
3. Remove remaining legacy business tables/files from active Culsans ownership.
4. Remove remaining legacy application routes/commands.
5. Rebuild all four products.
6. Install/run all four products.
7. Execute smoke gate.
8. Mark target architecture active.

There must not be a supported “Aster split but Iris/Echo still embedded” release state unless an emergency rollback requires it.

---

## Phase P5 — One Full Acceptance

Only now run the complete end-to-end suite.

The full suite is described in `09_TEST_AND_ACCEPTANCE.md`.

---

## Synchronization cadence

Use a short integration checkpoint whenever a stream completes a major vertical slice.

Checkpoint format:

```text
Repo:
Commit:
What moved:
What Culsans can now delete:
External activation actions implemented:
Data ownership changed:
Local tests:
Known blockers:
```

No long narrative status reports.

---

## Merge/cutover rule

The target is not “new app runs”.

The target is:

```text
new app owns the capability
AND
Culsans no longer owns the capability
```

A migrated capability with duplicate active ownership is not complete.
