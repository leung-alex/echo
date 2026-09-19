# Dependency Rules

Allowed high-level graph:

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

`echo-engine` may use only standard-library and domain-safe libraries. It must not depend on Slint, `rusqlite`, the Windows crate, desktop shell types, renderer APIs, or UI models.

`echo-storage` may depend on `echo-engine`, but not on Windows, Slint, presentation, or desktop. `echo-windows` may depend on `echo-engine` and platform libraries, but not on storage, SQLite, Slint, presentation, or desktop.

`echo-presentation` may depend on `echo-engine`. It must remain independent of Slint, Win32, storage, filesystem layout, renderer choice, and desktop worker types.

`apps/desktop` may depend on engine, storage, Windows, activation, presentation, and Slint. It is the composition root and UI-thread/worker boundary. It must not own deduplication, capture policy, FTS query construction, blob reconciliation, thumbnail policy, or native paste-target validation.

`apps/desktop/ui-crate` (`echo-desktop-ui`) compiles the Slint tree in
`apps/desktop/ui` and bundles its translations. Desktop re-exports these generated
types. The UI crate must not depend on any other Echo crate: business-only edits
must not invalidate its compiled artifact. Windows resources remain in the host
build script. Both crates retain the same development optimization level.

`apps/desktop/icon-crate` (`echo-icon-assets`) owns only immutable Lucide/system metadata
and SVG bytes. It must not depend on another Echo crate or Slint. Desktop adapts
individual SVGs to Slint images on the UI thread. The UI crate declares the image
callback without importing the asset crate, so catalog changes do not regenerate
the Slint component tree. Preserve original SVGs, persisted icon keys, and aliases.

Slint files may expose typed properties and callbacks, but must not reach storage, engine services, the named pipe, or Win32 directly. Slint component handles stay on the UI thread. Blocking work crosses `apps/desktop/src/service.rs` using typed Rust work and result messages; do not recreate command-name strings, JSON IPC, browser transports, or polling.

Interfaces live next to the engine behavior that needs them. Do not create repository-wide `common`, `helpers`, `utils`, `manager`, or `interfaces` dumping grounds.

Public engine, storage, Windows, activation, and presentation surfaces expose domain or adapter contracts rather than implementation modules. `echo_presentation::RowKey` is an opaque string at the desktop/Slint boundary; consumers must not parse it into a source or database ID.

Preview assets are bounded, binary-safe display derivatives. Copy and insert must resolve the engine item and use all retained original clipboard representations. History invalidation is event-driven. Capture persistence does not reconcile the whole blob store; cleanup belongs to the storage maintenance runtime.

The native instance transport is owned by `echo-windows`. It is scoped to the current user, logon session, and canonical data directory, and accepts only bounded validated activation arguments. The canonical activation flag is `--echo-activate`.
