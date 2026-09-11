# Memory50 lifecycle and evidence

Historical GPU comparison. The software-card implementation and current
acceptance contract are defined by [ADR 003](../architecture/ADR-003-software-cards.md).

The original GPU experiment targeted aggregate product Private Bytes strictly below 50,000,000
after hidden reclamation, without changing normal GPU Cover Flow or input safety.
Short hides retain warm resources for 30 seconds; reclamation should finish by
35 seconds. Visible and transient memory are separate results. The approved soak
duration is 600 seconds, not eight hours.

## Implemented resource boundaries

`--background` starts the existing single-instance shell and domain worker, then
waits on the typed event hub's condition variable before selecting a Slint backend.
Capture/storage/hotkeys remain on their existing owners. Coalesced invalidations
and bootstrap events are retained for first activation; explicit Quit wakes the
coordinator without creating graphics. This is initial lazy initialization, not
proof that a previously initialized backend can be destroyed.

Hidden trim invalidates preview/image/page generations, releases page payloads and
Slint image/model references, and retires offscreen renderer/compositor owners.
Reopening rebuilds those owners against the existing shared device. The main
Slint backend is retained; no repeated backend selection or device destruction is
attempted. Pending mutation/modal state prevents unsafe reclamation.
Normal dismissal publishes the new session epoch before arming the hidden timer.
The timer and worker acknowledgment must match both that epoch and the hidden
generation. The current software memory gate rejects a missing/late acknowledgment
even when the window successfully hides; visibility alone cannot establish reclamation.
An in-progress input transaction defers reclamation and retries within the same
hidden epoch/generation. The retry is discarded after a new activation. A busy
transaction can still miss the 35-second deadline; that remains a measured FAIL.
Automatic input-inspection fallback without a safe paste target keeps the
original input foreground. An explicit F6 request remains activating manual
History, including when it arrives before inspection completes. These origins
are carried explicitly rather than inferred from the notice text or pending flag.
The unavailable-input guard stays in the same session during an ordinary-target
probe; a verified target retires it before activating Echo, while copy-only
fallback retains Enter/Esc/F6 protection. Inspection timeout cancels the session
instead of opening a fallback that may race an unacknowledged or late provider.

Fuzzy search continues scanning the full space when its cache budget is exceeded.
Cache estimates include string capacities; overflowing candidate storage is
released during the scan. Original representations and storage durability are
unchanged.

Search candidates keep the full searchable document once, while ranked/list
results omit the duplicate editable body. Catalog rows do the same; Inspect
rehydrates the complete editor content, and execution still reads original
representations by ID. Full-document matching and pagination remain unchanged.

Worker display results use a 16 MiB weighted queue budget, including results
already dequeued for delivery. The producing worker may hold one additional
materialized result while waiting; control/activation/quit never wait for these
credits. An indivisible oversized editor result is delivered alone and explicitly
recorded as `oversized_display_result`, preserving the full content. Thus this is
backpressure, not a strict 16 MiB process/in-flight allocation ceiling or a P50
pass. Large-image transient allocations also remain separately reportable.

## Diagnostics and acceptance

`memory-diagnostics` is an attribution-only feature. With
`ECHO_MEMORY_NO_OFFSCREEN=1` it skips compositor and offscreen construction while
keeping the main GPU backend in that historical comparison. Current distribution
uses software-only builds with no default features, as specified by ADR 003;
diagnostic modes cannot substitute for current production acceptance.

For synthetic fixtures, `ECHO_WINDOWS_ACCEPTANCE=1` plus an existing
`ECHO_MEMORY_TRACE_DIR` enables bounded lifecycle JSONL without clipboard/query
content. Current maintained tools and their acceptance boundaries are listed in
[the performance README](../../tests/performance/README.md).

The original GPU matrix, process-split probes, 100-cycle runner and 4K/8K stress
runners are retired. The approved software workflow uses two five-minute T/M
runs through `Measure-SoftwareDeck.ps1`, plus separately scoped input and UI
regressions. The old three-session R50 GPU matrix is historical, not an additional
requirement for that workflow. Its evidence must not be relabeled as software PASS.

Process lifecycle subscriptions may require unavailable privileges. Preserve
UNAVAILABLE/Access denied and independently review ownership; missing metrics
are not zero, and SAMPLED_UNVERIFIED is not PASS. Current software rendering does
not split UI into another product process. The earlier M05 discussion is retained
in ADR 002 as decision history, not an active test command.
