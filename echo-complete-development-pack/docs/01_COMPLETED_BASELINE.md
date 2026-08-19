# Echo Completed Baseline (Stages E01–E05)

Treat product/code extraction as complete:

- Echo owns Clipboard platform/listener lifecycle.
- Echo owns representations, normalization, dedup/fingerprint and sensitive-source policy.
- Echo owns storage, blobs, reconciliation/GC, History, Favorites and Snippets.
- Echo owns Quick Insert retrieval/insertion orchestration and paste-target handling.
- Echo owns its own data directory/database/blob root.
- Echo runs as an independent Tauri process and can hide UI while its clipboard listener continues.
- Echo does not depend on Culsans source/runtime/storage/platform crates.
- Rust/domain tests exist across clipboard/storage/library/quick-insert.
- Current frontend test coverage is weaker than Culsans higher-level UI/system acceptance.

Do not add unrelated Echo product features during P06/P07.
