# Runtime styles and the UI build cache

`echo.cmd dev` generates current defaults and starts the compiled native application
with `ECHO_DEV_STYLE_SOURCE` pointing to `design/tokens/echo.tokens.json`.
Save changes to existing tokens marked `"runtime": true` to see them without
compiling or restarting. The debug-only worker checks every 500 ms. It validates
the complete document and applies a snapshot on the UI thread. Invalid saves keep
the last valid snapshot and report the field in the terminal. The worker stops
when the application exits. A second invocation forwarded to an already running
production instance cannot enable this worker; first quit that instance normally.

Runtime values cover colors, font sizes/weights, row and control sizes, internal
radii, spacing and row shadows. The row foreground default is `color.row-text`.
Search highlights use `color.search-text` and `color.search-background` from the
same snapshot. The native selection painter marks only matching UTF-8 ranges
without bolding text; high contrast retains system colors and bold emphasis. Theme changes
and valid edits restyle retained rows without issuing a new search or resetting
selection/scroll. High contrast uses inherited system text colors for matches.

Window/card exterior geometry, stage placement, cache budgets, pagination and
animation policy remain static. Unmarked values require regeneration and a build;
they do not change in the running application. New/deleted tokens, type changes
and runtime-flag changes also require a build. Remaining hardcoded Slint values
and structural edits still rebuild the UI.

## Generated boundary

- Slint receives typed runtime property declarations without their values.
- Presentation receives only static constants.
- Desktop receives `style_defaults.rs`, containing compiled values and typed
  setters. Release applies these directly before first display and contains no
  file reader or watcher. No JSON parser runs for release style initialization.
- The generator does not rewrite unchanged outputs. `echo.cmd build` and `dev`
  regenerate first; direct Cargo commands require `echo.cmd tokens` first.

Standalone Slint test fixtures must export their own `DesignTokens` global and
initialize it with `crate::style::apply_style!` and `StyleSnapshot::default()`
before rendering, as the row-click and side-layout fixtures do.

Keep Slint development optimization enabled. Runtime-value edits rebuild the
small desktop host when building a new executable; their UI artifact must remain
`fresh=true`. Hot edits within the running development application invoke no
compiler at all. Different Cargo profiles retain separate caches.

## Validation

During iteration, use `echo.cmd tokens --check` after generating outputs and the
focused token/style tests. Run canonical self-check, format check and full verify
once at completion. A full test-profile build is not required for every visual
edit. Record Cargo JSON artifact freshness and wall time for cache claims; native
style checks require isolated capture-disabled fixture data and native-test.

Runtime lengths accept finite values from 0 to 4096 DIP, font sizes must be
positive, and runtime integer weights accept integral values from 1 to 1000.
Colors accept `#RRGGBB` or `#RRGGBBAA`, with an optional dark color. Numeric values
cannot have a dark override. These checks apply to generation and hot loading.
