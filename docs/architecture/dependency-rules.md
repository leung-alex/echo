# Dependency Rules

Allowed high-level graph:

```text
echo-activation
       ^
apps/desktop -> echo-engine <- echo-storage
       |                ^
       +-> echo-windows-+
apps/ui -> desktop transport only
```

`echo-engine` may use only standard-library and domain-safe libraries. It must
not depend on Tauri, `rusqlite`, the Windows crate, WebView/frontend types, or
transport encoding.

`echo-storage` may depend on `echo-engine`, but not Tauri, Windows, React, or
frontend packages. `echo-windows` may depend on `echo-engine`, but not storage,
SQLite, Tauri, or frontend packages.

Desktop is a composition and transport shell. It must not own deduplication,
FTS query construction, thumbnail generation, blob GC, or native paste target
validation. Frontend feature code uses the typed wrappers under
`apps/ui/src/shared/ipc` rather than raw Tauri invokes.

Interfaces live next to the engine behavior that needs them. Do not create
repository-wide `common`, `helpers`, `utils`, `manager`, or `interfaces`
dumping grounds.

The engine/storage/windows/activation public surfaces expose domain contracts,
not adapter implementation modules. The desktop transport is the sole source
for checked-in TypeScript bindings, and frontend feature code reaches it only
through shared IPC wrappers. Preview resources are binary-safe and never use a
Base64 or data-URL transport. History invalidation is event-driven rather than
polling. Capture persistence does not reconcile the whole blob store; cleanup
is owned by the maintenance runtime.

The canonical activation flag is `--echo-activate`.
