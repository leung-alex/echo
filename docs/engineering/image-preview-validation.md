# Display-sized image previews: validation

Date: 2026-09-16. Implemented directly on local `main`, base `02120caa94508530b419869824d1c393c9ad2ba7` plus existing uncommitted UI/search/settings work. No commit or push in this change.

## Change

The main list reads the retained image representation by its source hash, then generates a Lanczos3 preview at the drawable physical pixel size. Small originals are not enlarged. Source identities also distinguish images whose 256px thumbnails happen to be identical. Persistent thumbnails, original clipboard payloads and storage schema are unchanged.

One dedicated decoder thread, a 32-request bounded queue, a maximum 1024px requested dimension, 32 MiB encoded input / 16 megapixel / 64 MiB decoder limits, and an 8 MiB incoming pixel cache bound the work. A page transition can temporarily retain the outgoing cache too. The existing hidden-window reclamation remains. Unsupported/oversized/corrupt sources fall back to their persistent thumbnail. No high-resolution disk cache is added. Cache misses can decode again after eviction.

Side cards initially use persistent thumbnails; a cached main preview can be reused there. Slint handles remain on the UI thread; the decoder sends typed pixel results. Shutdown stops the decoder and joins its thread after current bounded decoding finishes.

## Method and provenance

Evidence directory: `D:/Project/echo/.local/echo/preview-quality`.

- `before.exe`: SHA256 `2bb2fe2c2e8eed12008641797b2d3b3e226b4ed051c050dc873ff99ce8051fc2`.
- `after.exe`: SHA256 `9982ef37d685e46510047f8dff48f65eaf1c8ec1e5ca52c38d69935d6b33185d`.
- `before.patch`: pre-existing uncommitted work, captured before preview implementation.
- Both executables: native Windows Slint software renderer, Cargo dev profile, desktop/UI opt-level 1; `native-test` enabled only for isolated measurement executables. No concurrent build during final measurements.
- 24 synthetic PNGs cycling through 1200x760, 650x80, 350x35 and 3840x2160, with text and unique markers. Isolated data, capture disabled. Normal paging exposes 20 rows at a time.
- `tests/native/Measure-ImagePreview.py`: launch, wait for cached pixels, then two down/up scroll passes using input dispatched only to the owned Slint window. Memory sampled every 20ms. This is not physical mouse/keyboard acceptance.
- Runs `before/after-125-4..6`: three alternating pairs at 125%; values below are medians. `before/after-200` provides one additional 200% check.
- First-image time is launch to first nonzero cached image observed through the test bridge, not an exact first-frame trace. The first fixture is 4K. Settled time includes a deliberate 600ms stability wait and is not presented as decode latency.
- Render P95 measures render/present work in the software renderer; it is not end-to-end frame pacing or a universal 60fps claim.

## Measurements at 125%

| Metric | Before | After |
| --- | ---: | ---: |
| First cached image | 162.0 ms | 515.5 ms |
| Private memory peak | 38.03 MiB | 63.82 MiB |
| Private memory after scroll | 30.32 MiB | 31.04 MiB |
| Cold scroll render/present P95 | 28.02 ms | 26.62 ms |
| Warm scroll render/present P95 | 27.80 ms | 27.68 ms |

The main tradeoff is first-load decoding and transient memory. Steady memory and measured scrolling remained close in this fixture. The decoder is separate from the domain queue so image decoding no longer serializes searches behind itself; dedicated search-latency acceptance was NOT_RUN. Memory is not guaranteed below 50 MiB during decoding or at higher DPI.

At 200%, the single-run first-image times were 172.3 / 544.9 ms, peaks 50.29 / 76.34 MiB, and final private memory 37.09 / 39.46 MiB (before / after). Release performance, long soaks and physical mixed-monitor DPI changes were NOT_RUN.

## Visual evidence

PASS: the full 350x35 synthetic small original matches the rendered pixels exactly at both 125% and 200%. `pixel-check.json` records hashes and crop coordinates. The old displayed small screenshot shows enlarged, degraded letter edges. The new small screenshot occupies fewer display pixels because enlargement is disabled.

See `before-125-4/initial.png`, `after-125-4/initial.png` and `after-200/initial.png`. Large full-page images still reduce small text when fitted into a small card. These checks do not substitute for the user's visual review of their own screenshot.

## Checks

- PASS: `echo.cmd self-check`.
- PASS: `echo.cmd format --check`.
- PASS: `echo.cmd verify`, including exact small-original RGBA preservation, bounded resize and invalid input, plus original-source retrieval from History and Saved Items after moving the original entry. `verify.log` retains full output; opt-in physical tests remained ignored.
- PASS: `git diff --check`.
- PASS: `echo.cmd build` without `native-test`, 5.19s, reused UI crate.
- UI structural migration build: 7m13s. Subsequent host-only native-test build: 4.96s; decoder-lane host rebuild: 4.33s.
- User visual acceptance: PENDING. Normal debug application launched for review; portable/release packaging was NOT_RUN.
