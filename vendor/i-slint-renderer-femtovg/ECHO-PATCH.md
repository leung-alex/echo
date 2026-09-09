# Echo popup presentation latency

Source: crates.io `i-slint-renderer-femtovg` 1.17.1, upstream revision
`cf62c975c311e7036d599ed8ed0b7e6a8386a934`. The original manifest, VCS metadata,
copyright and license files are retained.

`wgpu.rs` requests one outstanding swapchain frame instead of WGPU's default
two. Echo is an interactive GUI with small GPU workloads; limiting queued work
reduces input-to-visible latency. FIFO/AutoVsync, transparent composition and
the application's session-bound cloak plus DWM synchronization remain enabled.
Resize preserves the setting, and surface/device recreation applies it again.
There is no new public API or environment setting.

Validation uses actual desktop pixels in `tests/native/Measure-PopupTiming.py`,
including independent process starts, repeated activation, both popup directions,
and native navigation recordings. An internal submit timestamp is insufficient
evidence. Check frame pacing on target hardware when upgrading this dependency.
