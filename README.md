# Echo Recall

Echo is the standalone reusable-content application extracted from Culsans.
Clipboard is the ingestion engine, Library contains History/Favorites/Snippets,
and Quick Insert is the retrieval and insertion surface.

The repository intentionally contains no dependency on Culsans source, runtime,
storage, platform crates, or data directories.

## Local development

```text
cargo test --workspace
pnpm --dir frontend/app install
pnpm --dir frontend/app test
pnpm --dir frontend/app build
```

On Windows, the native desktop package is built from `apps/desktop`.
