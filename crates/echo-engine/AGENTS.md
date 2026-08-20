# Engine Ownership

Business behavior belongs here. Read `docs/architecture/overview.md`,
`docs/architecture/dependency-rules.md`, and `docs/domain/glossary.md` before
changing ingestion, History, Saved Items, Quick Insert, preview requirements,
or settings.

Adapters implement the engine interfaces; the engine does not import storage,
Tauri, Win32, or frontend types.

For change locality, thumbnail bounds belong in `src/preview.rs`, Quick Insert
paste orchestration in `src/quick_insert.rs`, and search use-case contracts in
`src/history.rs`; see `docs/architecture/locality.md` before crossing an
adapter boundary.
