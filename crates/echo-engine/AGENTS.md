# Engine Ownership

Business behavior belongs here. Read `docs/architecture/overview.md`,
`docs/architecture/dependency-rules.md`, and `docs/domain/glossary.md` before
changing ingestion, History, Saved Items, Quick Insert, preview requirements,
or settings.

Adapters implement the engine interfaces; the engine does not import storage,
Tauri, Win32, or frontend types.
