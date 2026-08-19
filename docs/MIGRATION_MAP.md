# Echo Migration Map

## Scope and baseline

Echo is an independent product extracted from Culsans Clipboard, reusable content,
and Quick Insert. The implementation root is `D:\Project\echo`; the read-only
source root is `D:\Project\culsans`.

The inspected Culsans baseline is:

- branch: `main`
- commit: `e625306a8b23e3cc1326738e528612455f0a4db6`
- parent: `e823cebc9c99c9284d44f2774109bd5e40369763`
- source worktree status: clean except for the user-provided untracked
  `culsans-decomposition-development-pack/`
- local test baseline: `culsans-clipboard` 9 passed; `culsans-storage` 42
  passed, 1 ignored; Quick Insert runtime filter 6 passed

No Culsans source file is an Echo dependency. The source is reference material
only and remains untouched by Echo work.

## Ownership map

| Culsans capability | Echo owner | Migration note |
| --- | --- | --- |
| `WM_CLIPBOARDUPDATE`, source marker, sequence reads | `echo-platform-windows` | Echo owns the native listener lifecycle. |
| Clipboard snapshots and representations | `echo-platform` | Keep text, HTML, RTF, image, and file representations. |
| Clipboard normalization and HTML sanitization | `echo-clipboard` | Preserve fragment extraction, preview, content type, and source attribution behavior. |
| Fingerprint and deduplication | `echo-clipboard` + `echo-storage` | Preserve ordered representation SHA-256 and timestamp refresh semantics. |
| Inline data, blob files, hash validation | `echo-storage` | Echo owns `%LOCALAPPDATA%\Echo\blobs`. |
| Reconciliation, incremental GC, capacity eviction | `echo-storage` | Startup reconciliation runs in Echo background maintenance. |
| History and FTS search | Echo Library | History is one Library view. |
| Pinned/favorite saved insert items | Echo Library | Favorites are a Library capability, not a fourth module. |
| Snippets and groups | Echo Library | Snippets are the third Quick Insert view. |
| Quick Insert retrieval surface | `echo-quick-insert` and Echo UI | Quick Insert is a retrieval/insertion surface, not a data type. |
| Copy, target capture, paste, focus validation | Echo runtime/platform | Echo owns insertion orchestration and fail-closed target checks. |
| Clipboard settings and retention | Echo Library/settings | Preserve current behavior and limits. |
| Input Editor draft | Culsans shell | `input_draft` remains shell-owned; `draft_recovery` moves with it. |
| Input Editor Save as Snippet | Echo activation protocol | Culsans sends one-shot `echo.save_snippet`; it does not write Echo DB. |
| Activation and background lifecycle | Echo app | `echo.open`, `echo.quick_insert`, `echo.settings`, and `echo.save_snippet`. |

## Legacy data boundary

The only Culsans data migrated by Echo is Clipboard/reusable-content data:

- `clipboard_settings` row `id = 1`
- `clipboard_entries`
- `clipboard_representations`
- `clipboard_blobs` and every referenced file under `blobs\`
- `clipboard_fts` rebuilt from migrated entries
- `saved_insert_items` and `saved_insert_representations` when present
- `snippets`

The Culsans shell tables, drawing data, command/search data, and singleton
`input_draft` are not Echo data. Echo does not migrate or mutate them.

Legacy Culsans versions before migration `3002` have no saved-item tables.
For those databases, each `clipboard_entries.pinned = 1` row is converted into
an idempotent saved snapshot with all of its representations.

Before reading, the migration takes an exclusive migration lock and creates a
read-only backup snapshot of `culsans.sqlite3`, its `-wal`/`-shm` companions,
and `blobs\`. The snapshot is validated for row counts, foreign keys, FTS
coverage, SHA-256 blob names/content, and representation references. Echo
writes a migration marker only after all checks and destination writes succeed.
The source backup is retained. There is no permanent legacy read, write, or
dual-write path.

The inspected development fixture at
`D:\Project\culsans\.local\dev-data` contained 105 entries, 114
representations, 87 blob rows/files, zero missing blob references, zero
snippets, zero pinned entries, and no `saved_insert_items` tables. It is a
development fixture, not a user-data declaration. Production migration must
also support WAL-backed databases, pinned-only schemas, saved snapshots,
missing blobs, hash mismatches, and repeated runs.

## Verification and open seams

Each Echo ticket is tested locally and committed independently. Echo gates are
limited to Echo tests/build/typecheck and migration fixtures; unrelated Culsans
full regression is not run. Physical Windows clipboard/system acceptance stays
an explicit separately authorized gate.

The only deliberate cross-product seam is Input Editor snippet creation. After
activation v1 is enabled, Culsans sends `echo.save_snippet` with text/name/group
and Echo persists asynchronously. Culsans keeps draft recovery in its own shell
storage and must not open Echo SQLite or blobs.

