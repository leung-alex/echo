# Echo Architecture Overview

## Direction

Echo is a native Windows application with one business layer, explicit adapters, framework-independent presentation state, and a thin compiled Slint shell:

```text
                       echo-activation
                              ^
                              |
echo-storage -> echo-engine <- echo-windows
                    ^               ^
                    |               |
          echo-presentation         |
                    ^               |
                    +-- apps/desktop+
                           |
                  apps/desktop/ui (Slint)
```

`echo-engine` owns behavior and declares the interfaces it needs. Storage and Windows implement those interfaces. `echo-presentation` owns UI-independent state and interaction policy. `apps/desktop` composes the services, owns the native worker boundary, and binds state to compiled Slint components. It does not implement business policy.

## Ownership

The generated Slint code is compiled separately by `echo-desktop-ui` in
`apps/desktop/ui-crate`, with source still in `apps/desktop/ui`. Desktop business
changes reuse that dependency; UI or translation changes rebuild it normally.

Lucide and system icon metadata and original SVG bytes are compiled separately by
`echo-icon-assets` in `apps/desktop/icon-crate`. Its small resource loader exposes
the full catalog and individual SVGs. The Slint image callback requests only
instantiated icons; it does not construct a whole-gallery image array. Decoded
images are retained by the requesting component properties, without a separate
resident gallery cache. Business or layout edits do not recompile the asset crate.

### Engine

`ingest` normalizes clipboard snapshots and applies capture policy. `history` owns history queries and actions. `saved_items` owns the distinct durable Saved Item concept. `quick_insert` owns retrieval, target sessions, copy, and insert orchestration. `settings` owns settings values. `preview` owns preview asset requirements without UI encoding.

A Clipboard Item can retain multiple original representations, including text, HTML, RTF, images, and file lists. Copy and insert retrieve those stored original formats. Preview text and thumbnails are display derivatives and are never substitutes for the original clipboard payload.

### Adapters

`echo-storage` owns SQLite schema, FTS5 MATCH construction, blob files, migrations, persistence mapping, the writer actor, read connection, and maintenance runtime. `echo-windows` owns clipboard listeners and formats, target capture, focus validation, clipboard writes, paste delivery, HWND behavior, the native tray, and local named-pipe activation. Unsafe Win32 code is local to `echo-windows`.

`echo-activation` owns the bounded, versioned activation envelope and canonical `--echo-activate` flag. It does not own process transport or UI routing.

### Presentation and desktop

`echo-presentation` owns surface state, load generations, bounded result windows, selection, batch selection, keyboard/IME intent, session epochs, and opaque string row keys. Row-key encoding is private to that crate; desktop and Slint return keys without parsing them.

`apps/desktop/src/service.rs` runs blocking engine, storage, and preview work off the UI thread. Work and completions cross the boundary as typed Rust enums. `apps/desktop/src/app.rs` owns UI-thread coordination, generated Slint models, activation routing, stale-result rejection, and orderly shutdown. Slint component handles never leave the UI thread.

`apps/desktop/ui` contains the compiled Slint component tree. It renders History, Favorites, custom spaces, settings, About and dialogs in one native window. Navigation belongs inside each space card; there is no local search field. Inline Quick Insert uses the same non-activating native window while the query remains in the original editor; F6 opens unfiltered history for manual copying without a paste target. It does not call storage or native platform APIs directly.

## Runtime Flows

Clipboard ingestion is event-driven:

```text
Windows clipboard event
  -> echo-windows ClipboardSource
  -> echo-engine ingestion and policy
  -> echo-storage writer actor
  -> engine invalidation event
  -> desktop typed event
  -> echo-presentation reload generation
  -> Slint model update
```

Quick Insert activation captures and records the paste target before the Echo window is shown. A worker completion is accepted only for the active session epoch. Copy or insert then retrieves the retained original representations, writes them through the Windows adapter, and revalidates the target before paste delivery.

## Native Shell and Lifetime

Echo permits one resident host per Windows user, logon session, and canonical data directory. `ECHO_DATA_DIR` selects the data root when present; otherwise the desktop uses the user's local application-data `Echo` directory. The canonical data path and current user identity participate in the instance namespace.

A secondary launch validates and forwards bounded arguments over a local named pipe. The pipe is restricted to the current user, rejects remote clients, validates the peer process SID, and uses bounded transfer deadlines. It is not a TCP or browser automation endpoint.

The native tray can open Echo, Favorites, or Settings and can explicitly Quit. The tray icon is restored after Explorer recreates the taskbar. Closing or dismissing Echo hides its window while clipboard capture remains resident. Explicit Quit closes the event hub, exits the Slint loop, shuts down worker and storage activity, and then stops the pipe and tray hosts.

## Rendering and Windows Composition

The desktop uses Slint software rendering and a directly rendered alpha window buffer. A full content card and side cards with up to four real previews retain native DPI and share the current query. At most two full components participate in the carousel; cards move and expand or contract without scaling their content into images. No card screenshots, GPU textures or offscreen compositor are used in production. Static shadows and native clipping preserve the floating-card appearance. See `ADR-003-software-cards.md` for ownership, readiness and memory contracts; `cover-flow.md` records the retired GPU design.

The UI About view embeds Slint's `AboutSlint` component. Distribution attribution and license notices must match the exact versions resolved by Cargo; the presence of the component alone does not establish notice completeness.

## Storage and Search

Storage retains bounded FTS5 queries and stable cursors. Quick Insert uses Nucleo multiword matching over complete scoped metadata with revision-bound fuzzy cursors; a displayed page never defines the searchable corpus. The domain worker cooperatively abandons superseded scans. See `../engineering/inline-completion.md` for input protection and validation boundaries. No background UI polling is required. Storage runtime ownership, ordered schema migrations, instrumentation, reconciliation, and the deterministic storage diagnostic are documented in `docs/architecture/storage-runtime.md`.

Saved Items are the only durable reusable-item concept and remain distinct from History. Moving History into a space transactionally creates a Saved Item with its original representations and removes the source capture. Schema v9 gives each Saved Item exactly one owning space, enforced by a unique membership index. Favorites is a default collection, not an aggregate of custom spaces. Copying into another space creates an independently editable Saved Item and preserves original representations. Migration splits legacy shared memberships into independent items without changing names, payloads, tags or per-space order. Removing an item from a custom space moves it to Favorites. Deleting a custom space explicitly chooses between deleting all of its content (the default selection) and moving all content to Favorites; other spaces are unaffected.

Space navigation uses Tab and Shift+Tab while browsing; editors, settings, dialogs and IME keep their normal key handling. Schema v10 removes the retired space-switch shortcut preference from UI settings without changing other settings or content.
