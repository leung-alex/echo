# Echo Schema Migration Map

This document describes repository-owned upgrades for representative pre-R0, pre-R1, and pre-R2 Echo schemas. It is not an external import or compatibility runtime. The Slint migration changes the active desktop presentation and shell; it does not redefine stored clipboard or Saved Item data.

## Ownership Map

| Capability | Echo owner | Migration note |
| --- | --- | --- |
| Clipboard listener, source metadata, sequence reads | `crates/echo-windows` | Echo owns the native listener lifecycle. |
| Clipboard snapshots and representations | `crates/echo-engine` domain + `crates/echo-windows` adapter | Retain actual text, HTML, RTF, image, and file representations. |
| Normalization and HTML sanitization | `crates/echo-engine/src/ingest.rs` | Preserve fragment extraction, preview, content type, and source attribution behavior. |
| Fingerprint and deduplication | `crates/echo-engine` + `crates/echo-storage` | Preserve ordered representation SHA-256 and timestamp refresh semantics. |
| Inline data, blob files, hash validation | `crates/echo-storage` | Echo owns the local Echo blobs directory. |
| Reconciliation and capacity eviction | `crates/echo-storage` | Startup and debounced reconciliation run in Echo's maintenance runtime, not on each capture. |
| History and FTS5 search | `crates/echo-engine/src/history.rs` + `crates/echo-storage` | History is an engine use case backed by bounded storage queries and stable cursors. |
| Saved Item snapshots | `crates/echo-engine/src/saved_items.rs` + `crates/echo-storage` | Favorites are the UI label for the distinct durable Saved Item model. |
| Quick Insert retrieval and insertion | `crates/echo-engine/src/quick_insert.rs` + `crates/echo-presentation` + `apps/desktop` | Quick Insert is a retrieval surface, not a stored data type. |
| Copy, target capture, paste, focus validation | Engine interfaces + `crates/echo-windows` | Use retained original representations and keep target checks fail-closed. |
| Clipboard settings and retention | `crates/echo-engine/src/settings.rs` + `crates/echo-storage` | Preserve current behavior and limits. |
| Activation envelope | `crates/echo-activation` | Canonical flag is `--echo-activate`. |
| Single instance, tray, and native lifetime | `crates/echo-windows/src/shell` + `apps/desktop` | Scope the resident host to user, logon session, and canonical data directory; close hides and Quit shuts down. |
| Native UI | `crates/echo-presentation` + `apps/desktop/ui` | Presentation row keys are opaque strings; Slint is compiled into the desktop. |

## Schema Boundary

The ordered migrations preserve clipboard History, all original representations, referenced blobs, FTS5 data, settings, and Saved Item snapshots. For pre-R0 schemas without Saved Item tables, pinned clipboard entries are converted into idempotent Saved Item snapshots with all representations.

History and Saved Items remain separate models. A Saved Item survives source History deletion and may become independent after editing. The native UI migration must not flatten Saved Items back into pinned History or reduce an original multi-format payload to preview text.

The current schema opens idempotently. Normal runtime never scans an unrelated database, performs an external import, or chooses another user's data directory. `ECHO_DATA_DIR` may select an isolated synthetic data root for smoke or acceptance.
