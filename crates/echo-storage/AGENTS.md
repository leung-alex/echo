# Storage Ownership

This crate owns SQLite, FTS, blob files, migrations, and persistence mapping.
Read `docs/architecture/overview.md` and
`docs/architecture/dependency-rules.md`. Implement engine interfaces here;
do not move clipboard policy or native platform behavior into storage.

The writer runtime owns migrations and debounced maintenance; the reader
connection owns history/search reads and preview file access. Search SQL and
ranking stay in this crate, and capture paths must not reconcile the whole
store.
