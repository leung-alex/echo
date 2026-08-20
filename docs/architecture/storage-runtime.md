# Storage Runtime

R5 uses one dedicated storage writer and one dedicated read connection. The
writer is an actor with a bounded request channel; history queries and preview
file reads use the read connection and never wait on the writer actor. SQLite
WAL keeps committed writes visible to the read connection without introducing
a general-purpose pool.

Blob and thumbnail reconciliation runs on the storage maintenance runtime.
It runs once at startup and is debounced after delete, clear, capacity
eviction, and explicit repair requests. Normal capture does not scan either
directory. A thumbnail remains owned while any clipboard or Saved Item
representation references its original content hash.

## Schema Versions

`PRAGMA user_version` is advanced one step at a time:

- `1`: core Echo tables and legacy-compatible search placeholders;
- `2`: representation content identities;
- `3`: preview asset metadata;
- `4`: FTS5 search tables and rebuilt documents.

Opening an already-latest database is idempotent. The migration fixtures under
`crates/echo-storage/fixtures/migrations` cover pre-R0, pre-R1, and pre-R2
records. Legacy Culsans migration keeps its read-only backup behavior.

## Instrumentation

Operation metrics are structured as `{operation, samples, total_duration_us,
total_count}`. The operation names cover capture read, queue wait, dedupe,
database commit, blob write, thumbnail generation/write, history and Saved
Item query, preview open, maintenance reconcile, and first-result
availability. Metrics contain no clipboard text, file paths, or payload bytes.

## Deterministic Diagnostic

Run `.\echo.cmd perf`. It prints one JSON object with schema
`echo.r5.perf.v1` and fixed scenario fields for:

- 5,000 text rows, cursor page count, and exact-match count;
- 200 rows with 50 image rows;
- repeated large-image dedupe and blob-file count;
- referenced/orphan blob reconciliation invariants.

The command gates these counts and booleans before printing. It intentionally
does not gate wall-clock time, so the output is stable across machines.
