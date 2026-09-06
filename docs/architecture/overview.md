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

### Engine

`ingest` normalizes clipboard snapshots and applies capture policy. `history` owns history queries and actions. `saved_items` owns the distinct durable Saved Item concept. `quick_insert` owns retrieval, target sessions, copy, and insert orchestration. `settings` owns settings values. `preview` owns preview asset requirements without UI encoding.

A Clipboard Item can retain multiple original representations, including text, HTML, RTF, images, and file lists. Copy and insert retrieve those stored original formats. Preview text and thumbnails are display derivatives and are never substitutes for the original clipboard payload.

### Adapters

`echo-storage` owns SQLite schema, FTS5 MATCH construction, blob files, migrations, persistence mapping, the writer actor, read connection, and maintenance runtime. `echo-windows` owns clipboard listeners and formats, target capture, focus validation, clipboard writes, paste delivery, HWND behavior, the native tray, and local named-pipe activation. Unsafe Win32 code is local to `echo-windows`.

`echo-activation` owns the bounded, versioned activation envelope and canonical `--echo-activate` flag. It does not own process transport or UI routing.

### Presentation and desktop

`echo-presentation` owns surface state, load generations, bounded result windows, selection, batch selection, keyboard/IME intent, session epochs, and opaque string row keys. Row-key encoding is private to that crate; desktop and Slint return keys without parsing them.

`apps/desktop/src/service.rs` runs blocking engine, storage, and preview work off the UI thread. Work and completions cross the boundary as typed Rust enums. `apps/desktop/src/app.rs` owns UI-thread coordination, generated Slint models, activation routing, stale-result rejection, and orderly shutdown. Slint component handles never leave the UI thread.

`apps/desktop/ui` contains the compiled Slint component tree. It renders History, Favorites, custom spaces, settings, About and dialogs in one native window. Search and navigation belong inside each space card; there is no companion window. It does not call storage or native platform APIs directly.

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

The desktop normally uses a shared DX12 device for Slint FemtoVG-WGPU and Cover Flow, with a transparent DirectComposition swapchain. A persistent offscreen Slint card renders directly into cached GPU textures. Input never performs a CPU screenshot/readback/upload round trip. The front card is native, full-DPI Slint when settled; moving/side cards use bounded GPU textures. Software mode uses a clipped, flat native card rather than a black outer rectangle. See `cover-flow.md` for the native clipping, caching and economical-device policy.

The UI About view embeds Slint's `AboutSlint` component. Distribution attribution and license notices must match the exact versions resolved by Cargo; the presence of the component alone does not establish notice completeness.

## Storage and Search

History and Saved Item search use bounded FTS5 MATCH queries and stable cursors. No background UI polling is required. Storage runtime ownership, ordered schema migrations, instrumentation, reconciliation, and the deterministic storage diagnostic are documented in `docs/architecture/storage-runtime.md`.

Saved Items are the only durable reusable-item concept and remain distinct from History. Moving History into a space transactionally creates a Saved Item with its original representations and removes the source capture. Schema v6 adds spaces and membership/order relations; Favorites is a default collection, not an aggregate of every custom space. Sharing adds membership without copying payloads. Removing the last custom-space membership rehomes the item in Favorites. Deleting a custom space rehomes its exclusive items; deleting content everywhere remains a separate confirmed operation.
