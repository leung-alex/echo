# Change Locality

Use this map before changing cross-layer behavior. The primary module owns the rule; adapters and desktop translate it without copying policy.

## Thumbnail size

- Primary module: `crates/echo-engine/src/preview.rs`
- Adapter: `crates/echo-storage/src/lib.rs`
- Presentation consumer: `apps/desktop/src/app.rs`

Thumbnail bounds and encoding belong to the engine. Storage persists the content-addressed result. Desktop loads the bounded asset for a Slint model without replacing the original payload or reimplementing size policy.

## Quick Insert paste

- Primary module: `crates/echo-engine/src/quick_insert.rs`
- Adapter: `crates/echo-windows/src/lib.rs`
- Session state: `crates/echo-presentation/src/session.rs`
- Composition: `apps/desktop/src/app.rs` and `apps/desktop/src/service.rs`

The engine owns target/session and copy-versus-insert decisions. Windows owns native focus validation, clipboard-format writes, and paste delivery. Presentation owns activation epochs and stale completion rejection. Desktop captures the target on its worker before showing Echo and binds the result to Slint.

## History search ranking

- Primary module: `crates/echo-storage/src/lib.rs`
- Contract: `crates/echo-engine/src/history.rs`
- Presentation windowing: `crates/echo-presentation/src/lib.rs`

SQLite FTS5 MATCH construction, ranking, and cursor ordering stay in storage. The engine exposes the library contract. Presentation owns bounded result windows and load generations; desktop and Slint do not build SQL or materialize the full history.

- Adapter: `crates/echo-engine/src/history.rs`

## Row identity and selection

- Primary module: `crates/echo-presentation`
- Consumer: `apps/desktop/src/app.rs` and `apps/desktop/ui`

Presentation creates opaque string row keys and resolves them back to loaded engine items. Desktop and Slint may store, compare, and return a row key, but must not parse it to infer source or numeric identity. Selection, batch membership, and stale-load behavior stay in presentation.

## Clipboard formats

- Primary policy: `crates/echo-engine/src/ingest.rs`
- Native capture/write adapter: `crates/echo-windows/src/lib.rs`
- Persistence: `crates/echo-storage/src/lib.rs`

Echo retains the actual original representations for text, HTML, RTF, images, and files. UI preview text, thumbnails, or editable Saved Item text must not silently replace the original representations used by copy and insert.

## Single instance and activation

- Envelope contract: `crates/echo-activation`
- Named-pipe and instance transport: `crates/echo-windows/src/shell/pipe.rs` and `shell/mod.rs`
- Route/session composition: `apps/desktop/src/app.rs`

Activation envelope parsing belongs to `echo-activation`. User/session/data-directory scoping, peer identity checks, bounds, deadlines, and forwarding belong to the Windows shell. Desktop turns validated activation into a presentation session and route.

## Tray, close, and quit

- Native tray/window behavior: `crates/echo-windows/src/shell`
- Application lifetime coordination: `apps/desktop/src/app.rs` and `src/lib.rs`

Window close and Escape dismiss/hide surfaces. They do not stop clipboard capture. Only explicit Quit closes the event loop and initiates full worker, storage, clipboard, pipe, and tray shutdown.

## Renderer and Mica

- Renderer feature/selection: `apps/desktop/Cargo.toml` and `apps/desktop/src/lib.rs`
- Native composition effects: `crates/echo-windows/src/shell/window.rs`
- Visual fallback: `apps/desktop/ui/app-window.slint`

Software rendering is the default. GPU rendering is optional. Native Mica is conditional on renderer and Windows support; the Slint UI must retain an opaque fallback.
