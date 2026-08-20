# Change Locality

Use this map before changing a cross-layer behavior. The primary module owns
the rule; one adapter is the only allowed hop before desktop/UI transport.

## thumbnail size
- Primary module: `crates/echo-engine/src/preview.rs`
- Adapter: `crates/echo-storage/src/lib.rs`

Thumbnail bounds and encoding belong to the engine. Storage persists the
content-addressed result; desktop and UI consume metadata without reimplementing
the size rule.

## Quick Insert paste
- Primary module: `crates/echo-engine/src/quick_insert.rs`
- Adapter: `crates/echo-windows/src/lib.rs`

The engine owns target/session and paste decisions. Windows owns native focus
validation and delivery; desktop only composes the command boundary.

## History search ranking
- Primary module: `crates/echo-storage/src/lib.rs`
- Adapter: `crates/echo-engine/src/history.rs`

SQLite FTS5 MATCH construction, ranking, and cursor ordering stay in storage.
The engine exposes the library contract; desktop/UI transport does not build
search SQL or materialize the full history.
