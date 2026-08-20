# Echo Migration Map

Echo is an independent product extracted from Culsans clipboard and Quick
Insert behavior. The source product is reference material only; Echo never
opens Culsans runtime or UI code at build time.

## Ownership Map

| Capability | Echo owner | Migration note |
| --- | --- | --- |
| Clipboard listener, source metadata, sequence reads | `crates/echo-windows` | Echo owns the native listener lifecycle. |
| Clipboard snapshots and representations | `crates/echo-engine` domain + `crates/echo-windows` adapter | Keep text, HTML, RTF, image, and file representations. |
| Normalization and HTML sanitization | `crates/echo-engine/` ingest | Preserve fragment extraction, preview, content type, and source attribution behavior. |
| Fingerprint and deduplication | `crates/echo-engine` + `crates/echo-storage` | Preserve ordered representation SHA-256 and timestamp refresh semantics. |
| Inline data, blob files, hash validation | `crates/echo-storage` | Echo owns the local Echo blobs directory. |
| Reconciliation and capacity eviction | `crates/echo-storage` | Startup reconciliation runs in Echo background maintenance. |
| History and FTS search | `crates/echo-engine/history` + `crates/echo-storage` | History is an engine use case backed by the storage adapter. |
| Favorite saved snapshots | `crates/echo-engine/saved_items` + `crates/echo-storage` | Favorites are the UI label for Saved Items. |
| Quick Insert retrieval and insertion | `crates/echo-engine/quick_insert` + `apps/ui` | Quick Insert is a retrieval surface, not a data type. |
| Copy, target capture, paste, focus validation | engine interfaces + `crates/echo-windows` | Keep target checks fail-closed. |
| Clipboard settings and retention | `crates/echo-engine/settings` + `crates/echo-storage` | Preserve current behavior and limits. |
| Activation and lifecycle | `crates/echo-activation` + `apps/desktop` | Canonical flag is `--echo-activate`. |

## Legacy Data Boundary

The migration preserves clipboard history, representations, referenced blobs,
FTS data, settings, and saved-item snapshots. Legacy reusable-content tables
that are not part of Saved Items are not created, copied, queried, or kept as a
compatibility schema.

The Culsans shell tables, drawing data, command/search data, and `input_draft`
remain outside Echo. For older databases without saved-item tables, pinned
clipboard entries are converted into idempotent Saved Item snapshots with all
representations.

Before reading, migration takes its lock and creates a read-only backup
snapshot. It validates row counts, blob hashes, and representation references,
then writes a marker only after destination writes succeed. There is no
permanent legacy read, write, or dual-write path.
