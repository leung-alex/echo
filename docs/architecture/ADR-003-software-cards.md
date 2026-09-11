# ADR 003: Software rendering and space-card carousel

Status: implemented; local acceptance is partial (memory/function PASS, animation
timing FAIL). See [acceptance](../engineering/software-deck-acceptance.md).
Supersedes the production renderer choice in `cover-flow.md` and ADR 002.

## Decision and ownership

Default desktop and canonical build/run/acceptance/package commands use Slint
software rendering without `cover-flow`, WGPU or Skia. On 2026-09-11 the owner ended optional GPU/Skia diagnostic maintenance.
Those implementations and developer features are retired; their last retained
baseline is `44fc2a0a723595a3ff51ca557bfa03653147e521`. Public activation and
`--echo-activate` are unchanged. One resident process retains the storage writer,
reader, capture and input execution ownership.

`echo-presentation::slide` owns finite transitions and one last pending target.
The desktop owns at most two full Slint row models; the outgoing model is frozen
and each side retains at most four read-only preview rows. The incoming space/session/query/revision
and a renderer completion must match before motion. Insertion, selection and
content changes remain blocked while loading/moving. Escape/navigation remain.

Following the user's visual correction, side cards use 70% of the main-card
width (448 DIP beside a 640 DIP main card), matching the old visible proportion.
They remain 12 DIP from the main card and 32 DIP shorter. The incoming side card
moves to center and expands; the outgoing main card moves to the opposite side
and becomes a small preview. Neighbor cards leave/enter the carousel. Complete
content keeps its final layout size and is clipped/revealed, never scaled into a bitmap.
Narrow windows hide them and retain header navigation and shortcuts. Speeds are
140/180/220 ms cubic ease-out. Geometry advances before each scheduled frame;
loading is event-driven, with no animation polling timer. The redraw scheduler
uses absolute deadlines with a 60 Hz ceiling. A scoped
Windows timer-resolution lease runs during motion only. Reduced/off/system-disabled
motion switches directly after content is ready.

The user's later correction restores real side-card previews and concurrent
filtering of the center and both neighbors from the original input query. Each
neighbor search still scans complete documents, but materializes only its first
four results. Preview replies must match the query generation, neighbor identity
and storage revision. Previous previews are marked Filtering while replacements
load. Original payloads are never truncated; only read-only preview text is elided.
Hidden and superseded results cannot populate these models. Side thumbnails share
the same byte budget, including references held by moving cards.

## Software presentation and first frame

The pinned Slint software renderer ignores its GPU drop-shadow property. Static
shadows use rounded translucent rectangles. `echo-windows::SoftwareFrame` owns
one premultiplied BGRA DIB and an owned same-thread HWND. Slint writes directly
into this final window buffer; `UpdateLayeredWindow` presents its alpha. There
are no card bitmaps, screenshots, readbacks or intermediate compositor outputs.
Winit's warm-show style reset is handled without reallocating that buffer,
moving the window or changing input focus. Software frame acknowledgments follow
the render/present call and thumbnail requests emitted by layout.

Native shape and Quick Insert placement use two-dimensional card rectangles.
The input target, caret anchor, epochs and original-format execution remain
authoritative. No UI-process split is introduced.

## Budgets and reclamation

Initial display-data budgets are 4 MiB search, 4 MiB thumbnails and 4 MiB page,
model and in-flight data. Rust string/vector capacities are charged directly.
Each fresh row records actual System allocation requests, including
opaque Slint styled-text/string buffers. The allocator remains System, with a
thread-local counter active only around fresh row assembly. Charges follow retained
rows through keyed reconciliation, and the owned vector's capacity is counted.
The thumbnail budget reserves 2 MiB each for incoming and outgoing images; cache
eviction cannot erase charges for pixels still held by the departing panel. This measurement
excludes allocator bookkeeping, widget/layout caches, the final framebuffer and
Windows/DWM memory. All product Private Bytes remain in the separate 50 MB gate;
these buffer counters do not prove a process allocation high-water mark.

Page batches use byte limits and retained cursors. A search corpus exceeding
its budget is scanned completely with cancellation. Original payloads remain in
storage. Large explicit editor payload exceptions are traced and cannot certify
a bounded display-cache or memory PASS.

Short hides retain warm resources. After 30 seconds, epochs advance, display
models/images/framebuffer are released and the worker acknowledges search-cache
reclamation. Completion must occur by 35 seconds. Capture, hotkeys and storage
continue. Late results cannot recreate hidden display resources.

## Compatibility and acceptance

Persisted `cover_flow` means the card carousel, `flat` means static cards and `auto`
uses software; schemas/settings are not bulk rewritten. GPU/reflection and
side-content controls are removed; the other settings remain.

Acceptance uses a frozen actual software Release baseline and two independent
five-minute ordinary synthetic runs: text and exactly 20 1920x1080 images.
Both stable foreground and hidden-after-35-second Private Bytes must be strictly
below 50,000,000. Identity-bound raw samples, native input/visual evidence and
hashes accompany the report. Sampled peaks are not proven allocation high-water
marks. Physical IME and unavailable monitors/DPI remain separately reported.

References: [Slint renderer limitations](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/),
[Windows layered windows](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features).
