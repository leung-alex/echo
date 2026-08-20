# Desktop Ownership

Desktop is the composition root. Read `docs/architecture/overview.md` and
`docs/architecture/dependency-rules.md`. Keep commands, events, activation,
and transport DTO mapping here; business policy belongs to `echo-engine`.

Desktop does not schedule storage startup maintenance or build search/preview
payloads. The Rust transport module is the source for generated UI bindings.
