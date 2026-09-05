# Presentation Ownership

`echo-presentation` is Echo's UI-framework-independent presentation layer. It may depend on `echo-engine`, but it must not depend on Slint, Win32, SQLite, storage, desktop shell types, or renderer APIs.

It owns:

- surface query state and load generations;
- bounded result windows, forward cursors, and previous-window navigation;
- stable selection and batch-selection behavior;
- keyboard and IME-aware interaction intent;
- Quick Insert activation/session epochs and stale-completion rejection;
- bounded recent activation request IDs;
- opaque string `RowKey` values used by UI callbacks.

`RowKey` is the presentation boundary for row identity. Desktop and Slint may retain and return the string, but must not parse it to recover a source or numeric database ID. Resolution back to an engine item stays in presentation-owned state.

This crate does not load thumbnails, touch the clipboard, perform persistence, create native windows, or schedule worker threads. Those effects are coordinated by desktop using engine and adapter services.
