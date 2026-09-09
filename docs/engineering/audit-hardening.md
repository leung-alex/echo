# Audit hardening and regression ownership

Base: `6ee0b5718ccb3acaba371b9631d74683cd9dd8ab` on `main`.
This change keeps Rust + Slint, the current card design, and the search semantics.
No dependency upgrade, database migration, or user-data deletion is involved.

## Correctness changes

- Clipboard reads and writes have separate opening paths. Writes use the retained clipboard-source HWND, not a null owner.
- Every supported format is validated and allocated before EmptyClipboard. HGLOBAL allocations are RAII-owned until successful transfer to Windows. Publication errors remain errors; this does not claim an atomic rollback across several SetClipboardData calls.
- Capture checks ExcludeClipboardContentFromMonitorProcessing and CanIncludeInClipboardHistory before reading the payload. Missing markers allow capture; exclusion presence, a zero history DWORD, or an invalid history value do not. Cloud-only exclusion is independent.
- All inline events use the event-loop proxy path. Adjacent observations coalesce, control events remain barriers, and the UI independently rejects regressing revision/input-serial values.
- Creating, duplicating, or moving a Saved Item allocates a signed front-order key inside its transaction instead of updating every retained row. Existing row keys/payloads remain unchanged, pagination accepts signed keys, and explicit reorder keeps its established behavior. Exhaustion fails without modifying content.
- The UI drains at most 32 queued events per turn without discarding accepted work completions or Quit.
- Unfiltered space pages reuse the already-read membership count in the same WAL snapshot. Tuple cursors and ordering match the existing membership composite index; repeated metadata statements are cached. Filtered totals and full-corpus fuzzy matching are unchanged.
- Dismissal and new activation synchronously invalidate running searches. Session retirement drops query/result-page state, while bounded revision-checked corpora follow the existing hidden-trim timer. A stale trim cannot clear a newer session.
- A transient native focus transition to the original top-level root is unknown, not a verified editor or a definite different editor. Confirmation is disabled until exact editor revalidation; other controls remain rejected.
- Compatibility and error status is rendered in the active card rather than existing only in internal state.

## Test entry points

`echo.cmd self-check`, `echo.cmd format --check`, and `echo.cmd verify` remain the non-mutating developer gates.
`echo.cmd smoke` checks one native window, searchless manual History, close-to-hide, reopening, and graceful exit using synthetic data.
With explicit `ECHO_WINDOWS_ACCEPTANCE=1`, `echo.cmd acceptance clipboard` runs native text/HTML/RTF/BMP/file-list roundtrips, exclusion policies, and actual manual-row copying.
`echo.cmd acceptance quick-insert` runs the current owned-input suite against a separate optimized native-test executable. It does not depend on a browser being installed.
Both mutation gates materialize the original clipboard in memory and restore it, including on test failure. Payloads are not included in preservation reports.

The explicit scale gate is:

```powershell
cargo test -p echo-storage --test fuzzy_search fuzzy_search_large_corpus_keeps_tail_results_and_cancels --locked -- --ignored --nocapture
```

It exercises real 1k/10k SQLite corpora, including the streaming fallback beyond the 16 MiB search budget, retained oldest-item completeness, session reopening, and cooperative cancellation. Printed cold/reopen/cancellation durations are measurements, not a desktop-latency guarantee.

## Acceptance boundaries

- Production packaging must never use `native-test`; the test executable is a separate artifact.
- Native-test fault injection, real installed IME driven by synthetic keys, and isolated browser fixtures are not manual physical-keyboard or actual ChatGPT/Codex draft certification.
- The old inline runner expected a removed local search field, removed Insert button, and automatic per-query shrinking. Current main instead uses searchless F6 browsing, row-click insertion, and a session-latched height. The updated cases explicitly identify those current contracts; they do not certify auto-shrinking.
- Tests must retain failed attempts and write new evidence directories on retry. A successful build is not a passed foreground test.
- Network or tool-delivery failure does not establish whether a command ran. Inspect device identity, owned process IDs, file diffs, exit files, and durable logs before repeating side effects.

## Vendored runtime maintenance

The repository already includes per-patch records. Preserve these sources rather than creating a second patch policy:

| Runtime seam | Existing record | Upgrade checks |
| --- | --- | --- |
| Slint 1.17.1 backend | `vendor/i-slint-backend-winit/ECHO-PATCH.md` | Accessibility values, non-activating input, offscreen adapter factory |
| Slint 1.17.1 WGPU renderer | `vendor/i-slint-renderer-femtovg/ECHO-PATCH.md` | Frame pacing, transparent composition, actual popup pixels |
| FemtoVG 0.25.1 | `vendor/femtovg/ECHO_PATCHES.md` | Font identity caching and retained pipeline regressions |

`private_unstable_api` remains version-pinned. No runtime upgrade or speculative renderer rewrite is included in this correctness repair.
