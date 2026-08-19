# Codex Prompt — Echo Extraction Agent

Workspace: `D:\Projects\echo`

Source reference: read-only access to `D:\Projects\culsans`.

Read the architecture pack and `docs/07_ECHO_PLAN.md`.

Mission:

Build Echo as the independent Clipboard/Recall/Quick Insert product.

Before editing:

- inventory clipboard listener, storage, blobs, History, Favorites, Saved Insert Items, Snippets, Quick Insert, paste target behavior, settings and tests;
- write `docs/MIGRATION_MAP.md`;
- record the Culsans baseline SHA.

Hard rules:

- Echo owns the clipboard listener;
- Echo owns `echo.sqlite3` and `Echo\blobs`;
- no permanent reads/writes to Culsans DB after migration;
- Quick Insert is the main insertion surface, not a fourth content domain;
- paste target validation belongs to Echo;
- no sibling source dependency;
- implement activation v1.

Definition of Done:

Echo builds/runs/tests without Culsans source, migrated data is preserved, and Culsans can remove clipboard/Quick Insert ownership.
