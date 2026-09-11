# ADR 002: Memory50 lifecycle and the process-split gate

Status: single-process reclamation implemented; UI-process split not promoted.

Historical GPU decision. Optional GPU/Skia implementations were retired on
2026-09-11; the lifecycle evidence below is preserved, not an active GPU gate.
Production rendering is superseded by
[ADR 003](ADR-003-software-cards.md); prior evidence below is retained.

## Contract

R50 is the sum of Private Bytes of all live Echo product processes, strictly
below 50,000,000 bytes after hidden reclamation. Hiding starts a 30-second warm
retention interval; reclamation must finish by 35 seconds. Moving memory to
another product process, reducing image quality, or substituting working set
cannot satisfy this contract.

## Current decision

The resident worker starts capture, storage and activation before lazy Slint
initialization. After a long hide, the desktop invalidates display generations,
releases models and image references, retires the offscreen renderer/compositor,
and awaits the matching search-cache acknowledgment. The Slint main backend and
shared GPU device remain alive. Reopening uses that same device and revalidates
the session before presenting content. Short hides retain warm resources.

This removes application-owned retention but does not establish R50. Intermediate
measurements found the capture-enabled core below the threshold in a short text
scenario, while the initialized GPU process remained above it after acknowledged
reclamation. A separate large-image diagnostic also found retained memory after
Windows clipboard conversion; the short text result is not a universal core pass.
Raw evidence and exact binary hashes live under the independent Memory50 evidence
run, rather than the immutable development pack.

M05 therefore evaluates an exiting UI child. M06 is **NOT_PROMOTED**, not
**NOT_NEEDED**: the isolated prototype is a render/lifecycle experiment with input
actions disabled. It cannot pass the required input-safety gate or be packaged as
the normal product. Its first runtime probe also observed the core and child exit
after the initial handshake, before the hidden-exit/restore sequence completed;
that probe is a lifecycle FAIL and supplies no latency pass. Single-process delivery must continue to report any R50 or
latency failure without changing the threshold.

## Prototype boundary, not an adopted product protocol

The isolated prototype keeps the public executable and `--echo-activate` entry.
The resident worker exclusively owns storage, capture, hotkeys, original payloads
and input targets. A private child owns Slint and WGPU and never opens storage.
Its versioned binary pipe uses an `EOUI` header and bincode payloads, with 64 KiB
control and 1 MiB data limits and 4 MiB in-flight credits. Admission binds the
user/session, kernel peer PID, creation time, nonce and UI epoch; display frames
also carry the independent core session epoch. HWNDs, COM interfaces and original
clipboard bytes are excluded from display messages.

The prototype can render rows and settings and exit after hidden shutdown. Copy,
insert, inline replacement, mutations, thumbnail transfer and interactive request
routing are not complete. Production promotion requires those paths, at-most-once
execution, stale-message rejection, complete first frames, the preregistered
latency tolerances, and all-process R50 evidence. A pipe disconnect alone never
proves that a child exited or released its memory.

## Consequences

The ordinary build preserves the existing single-writer/single-reader ownership
and retained original formats. Diagnostics and `native-test` stay out of portable
distribution builds. Baseline artifacts and synthetic test evidence are retained;
rollback replaces the executable and does not delete real History or Saved Items.
