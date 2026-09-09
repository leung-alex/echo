# Echo rendering fixes

Vendored library source from crates.io `femtovg` 0.25.1, matching the Slint 1.17.1 dependency. Upstream MIT and Apache notices are retained. `Cargo.toml.orig` preserves the upstream manifest; example targets and their development dependencies are omitted from this library-only copy.

- `src/text/font.rs`: retain the Swash face offset and cache key for each immutable font. Constructing a new `FontRef` for each glyph assigned a fresh identity, preventing font and hinting caches from working. Font data, face indices, rendering mode, and cache capacities are unchanged.
- `src/renderer/wgpu.rs`: keep pipelines across 128 unused flushes. Slint submits the background clear separately from content, and cached layers can go dozens of frames without their shadow/content passes. Early eviction recompiled those pipelines on the first popup resize. Entries still expire and are released with the renderer/device; card and texture budgets are unchanged.
- `src/lib.rs`: gate the upstream `FilePath` import with the feature that uses it.

Regression commands (from the Echo root):

```powershell
cargo test -p femtovg --lib cache_identity_tests --offline
cargo test -p femtovg --lib clear_and_content --offline -- --ignored
```

The second test explicitly requires a GPU. It renders real clear/content passes, compares retained WGPU pipeline handles through repeated frames, and checks eviction of idle content pipelines. It is not desktop-visibility evidence.
