# Echo Schema Migration Map

This document describes repository-owned upgrades for representative pre-R0,
pre-R1, and pre-R2 Echo schemas. It is not an external import or compatibility
runtime.

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

## Schema Boundary

The ordered migrations preserve clipboard history, representations, referenced
blobs, FTS data, settings, and Saved Item snapshots. For pre-R0 schemas without
Saved Item tables, pinned clipboard entries are converted into idempotent Saved
Item snapshots with all representations. The current schema opens idempotently;
normal runtime never scans an unrelated database or performs an external import.
