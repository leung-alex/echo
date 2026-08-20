# Storage Ownership

This crate owns SQLite, FTS, blob files, migrations, and persistence mapping.
Read `docs/architecture/overview.md` and
`docs/architecture/dependency-rules.md`. Implement engine interfaces here;
do not move clipboard policy or native platform behavior into storage.
