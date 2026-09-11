> Historical GPU design. Production behavior is defined by ADR 003; optional GPU/Skia implementations were retired on 2026-09-11. See RETIREMENT_REPORT.md at the repository root.

# Space Cover Flow

## Product and identity

One native `AppWindow` presents History, Favorites and user-created collection spaces. The search field, space title, local toolbar, content list and navigation footer are inside the card. Settings is a separate two-dimensional card, never a fake collection in the deck. The stage outside cards is transparent; no outer Mica panel or window border is painted.

History is `SpaceId(1)`, Favorites is `SpaceId(2)`, and custom collections have persistent positive i64 IDs. IDs cross Slint as opaque strings, never narrowing to a UI integer. A `SavedItem` is the only reusable payload. A membership associates a Saved Item with a space and its local order. Favorites is not an automatic aggregate of custom collections.

The v6 migration assigns existing Saved Items to Favorites. Space mutations carry expected revisions and bounded request identities. Scoped cursors cannot cross spaces or revisions. Deleting a custom space rehomes exclusive content to Favorites atomically; removing a membership is distinct from confirmed deletion everywhere.

## Rendering path

- `apps/desktop/src/graphics.rs` creates one WGPU 29 device/queue and gives that same configuration to Slint. Windows uses DX12 `DxgiFromVisual` and a no-redirection-bitmap window for alpha composition.
- `cover_flow/offscreen.rs` owns one persistent `CardSnapshot` with a public Slint `FemtoVGWGPURenderer` and a custom public `WindowAdapter`. It has no HWND or accessibility provider. It renders into a GPU texture, not into CPU pixels.
- `vendor/i-slint-backend-winit/echo_offscreen.rs` is a small opt-in, RAII-scoped adapter-factory hook needed to associate a compiled component with that adapter. It is version-pinned with Slint 1.17.1; engine/storage never depend on it.
- `cover_flow/compositor.rs` owns reusable panel targets, bind groups, uniform buffers, one output texture/view and one cached Slint Image. WGSL supplies true perspective, Y rotation, analytic shadows and an optional subtle lower-edge reflection.
- `cover_flow/bridge.rs` only submits a prebuilt scene in the rendering notifier. Unchanged scenes do not redraw. No production card path calls `take_snapshot`, GPU map/readback, or blocking device polling.
- The front card uses native Slint controls at full window DPI when settled. During motion its tree is frozen while textures move; updates are handed back to the native card after settlement. The real search field keeps its input focus.

`Window::take_snapshot` is used only by the explicitly enabled native test bridge and the standalone diagnostic probe. It must never return to the production navigation path.

## Resource policy

Standard moving-texture budget: 48 MiB, maximum raster scale 2, frame cap 144 Hz. Economical policy: 32 MiB, maximum raster scale 1.5, cap 60 Hz, no reflections. Device texture-dimension limits also constrain allocation. Four panel targets plus the output are reserved in the pixel-budget calculation; the compositor additionally rejects allocations over the 64 MiB hard limit.

The economical policy is chosen for integrated GPUs or configured battery simplification. Raster scale is bounded and quantized downward; it affects moving textures, not settled text. A test-only environment override can exercise this policy on a discrete GPU, but that is not an integrated-GPU hardware benchmark.

GPU numbers exclude driver allocations, glyph caches and process memory. Report private working set/process memory independently. UI models, previews, thumbnail pixels and texture caches are bounded. Hidden windows stop deck/preview animation, and the configured 30-second hidden timer releases rebuildable motion caches.

## Keyboard and insertion safety

`Deck` separates requested, presented and interactive space identity. Repeated Tab retargets the analytic critically damped spring; it does not queue obsolete animations. Enter finishes to the latest target and checks ready content before taking an action. Missing content produces a retry message, never a deferred paste. IME composition, editors, modal dialogs and actual control focus retain their native keys. F6 switches navigation/control focus; the shortcut can be changed to Ctrl+Tab.

An external Quick Insert target is captured before showing the single window. The worker validates session epochs and original item identity. Copy/paste never uses card images, display text or preview data. Graphics recovery restarts only in manager mode and never replays a pending activation or insert.

## Native shape and transparent composition

`echo-windows::shell::card_window` is the only owner of GDI regions and DWM chrome calls. Region ownership transfers to Windows only after successful SetWindowRgn; temporary regions have RAII cleanup. A stable deck clips/hit-tests near its projected cards. An animating deck temporarily removes the binary clip so moving cards are not cut off. Cached identical shapes do not issue GDI calls each frame. Software mode clips to the exact central rounded card. DWM is told not to paint an outside border, rounded outer window or backdrop.

Region, viewport and scroll changes are separate: native clipping emits window-position notifications, but notifications with unchanged logical geometry and scroll do not invalidate content textures.

## Settings and tokens

Appearance, spaces, keyboard, capture/privacy and storage/diagnostics use a draft SettingsPatch. Save validates and atomically persists all settings with an expected revision. Cancel/discard restores persisted values; theme preview is reversible. Graphics preference changes require a safe restart. Automatic last-space writes do not overwrite a form's revision or get lost when it is saved.

`design/tokens/echo.tokens.json` is the single source for CSS, Slint and Rust design outputs. Run `echo.cmd tokens` to regenerate and `echo.cmd tokens --check` to reject drift. CSS is a design/reference artifact, not a runtime dependency or browser styling engine.

## Test boundaries

`native-test` enables an owned-window input/capture bridge only with ECHO_WINDOWS_ACCEPTANCE=1, a canonical evidence root, and a capture-disabled synthetic fixture inside that root. It has no TCP endpoint, cannot dispatch arbitrary commands and cannot send global keyboard input. Normal release builds do not contain the bridge. UIA drives actual controls; PNGs come from the owned Slint renderer, never unrelated desktop content.

Measure optimized builds, recording the binary SHA, data fixture, scale, policy, renderer and actual adapter/backend. Render-callback CPU duration is not GPU execution time; callback spacing is not an independently measured display presentation time. Report those distinctions. Simulated budgets and scale factors do not certify physical iGPU hardware, mixed-DPI monitors or physical Chinese IME. Archive PASS/FAIL/NOT_RUN from actual executions rather than inferring them from compilation.

## Projected-card image quality

Card-cache resolution is separate from the output resolution. `RasterPolicy::panel_scale` prefers a 2x cached card under the standard policy or 1.5x under the economical policy, then clamps it against the same output-plus-four-card memory reservation and device dimension limit. It does not increase the whole-window output scale. Resolutions are reallocated only when dimensions/policy change; dirty cards are rendered on the shared GPU, with no CPU screenshot round trip.

The panel mesh includes a small outward fringe. The fragment shader evaluates the rounded-rectangle signed distance and its screen-space derivative before branching, and converts that distance into pixel coverage. An unexpanded quad would clip away the outside half of the antialiasing fringe. This analytic edge treatment does not require a multisampled full-window target.

The shader integrates four bilinear samples over the projected output-pixel footprint when minifying the high-density card. This preserves small-stroke coverage more consistently than one sample from a 1x screenshot. It is a bounded four-tap filter, not a mipmap or anisotropic-sampler claim, and not a post-processing sharpening filter. Settled foreground controls remain native full-DPI Slint. Tiny text in a substantially compressed side card is still limited by its final physical pixel size.

Quality controls are generated from `quality.card-raster-standard`, `quality.card-raster-economical`, `flow.edge-aa-pixels`, and `flow.edge-padding-pixels`. Native-test diagnostics expose actual panel texture dimensions and the output raster scale so screenshots and allocation claims can be checked against the running binary.
