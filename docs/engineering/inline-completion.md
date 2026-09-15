# Inline completion lifecycle and evidence

Inline Quick Insert retains focus in the external editor and replaces only the
session's original query span. Echo has no local search field. F6 retires the
inline session and opens unfiltered history for manual copying, with no paste
target. Initial input inspection failures may retry ordinary Quick Insert using
the still-current pre-show snapshot; an unverified target remains copy-only.
Failures after inline activation never authorize a new ordinary paste target.
History and Saved Items retain their distinct ownership and original payloads.

Writable ValuePattern-only inputs enter ordinary Quick Insert directly after
focus, process, ownership, and runtime identity validation. The captured target
crosses the typed activation boundary without a second post-failure capture.
This mode does not filter from external text or replace a query span. Selection
pastes once into the original input through the existing ordinary paste adapter;
delivery revalidates that same input. Read-only, password, unidentified, and
non-editable controls remain ineligible. No application-name allowlist is used.

When UIA returns an unfocused host or misreports keyboard focusability, ordinary
paste can resolve MSAA's direct focus chain in the captured native focus window
and at most 32 descendant HWNDs. Multiple distinct focused inputs are rejected.
The MSAA input must report focused, focusable, writable, non-password text; its
converted UIA element still requires input focus, writability, process ownership
and a stable runtime identity. Capture and delivery use the same resolver. A
TextPattern synthesized by this bridge does not enable inline replacement.

Writable UIA Group composers (including Feishu chat) may enter inline mode when
their TextPattern exposes an editor-scoped document and selectable range. Group
is not sufficient by itself: focus, ownership, visibility, writability, range and
composition checks all remain mandatory. The popup stays non-activating and the
query is typed in the original composer. Selection readback still gates actual
replacement; advertised TextPattern support is not a compatibility acceptance.

Editor Kit's empty Group composer exposes a read-only placeholder plus three
zero-width leaves. Only that verified tree and collapsed caret are projected to
the stable empty context; literal text is not trimmed. Otherwise the disappearing
placeholder would suspend the first query observation. Multiline paste readback
accounts for the provider's zero-width paragraph separators without changing the
clipboard payload or relaxing the preserved prefix/suffix checks.

## Input ownership

The Windows adapter owns the keyboard hook, scoped editor and replacement. It
arms the hook before publishing `Started` or a protected `Unavailable` surface.
The hook's `GuardLease` is independent of the mutable callback state so a
reentrant callback can still consume Enter. Consumed key sequences retain their
key-up ownership while the hook retires. Re-activation obtains a new Windows hook
registration acknowledgement; a responsive thread alone is not proof of an OS hook.
The new registration is acquired before the old one is released; observer setup
failure cannot discard an outstanding consumed key sequence.
The UI hides the inline window before releasing that lease. Retirement uses an
atomic request with a hook-thread watchdog, so a full ordinary queue cannot lose
it. Enter sequence ownership and repeat state also survive callback reentrancy.

`key_policy::decide_key` is a pure decision seam. Filtering, no results, unknown
composition and modified confirmation keys cannot fall through to host submission.
Text/selection invalidation revokes result execution. Caret SHOW/HIDE and geometry
events do not count as text edits. A failed provider read preserves protection;
a positively observed different editor cancels the old session.
Repeated same-editor focus events request observation without declaring a text
mutation. Missing foreground/native focus identity is unknown and gets bounded
retries before suspension; it is not evidence of a different editor.

Inline window style is reconciled after show and preserves `WS_EX_NOACTIVATE`
through framework style updates. Pointer actions do not request a Slint focus
change. Manager/F6 mode restores the prior native style and normal focus behavior.

`CompositionEvidence` binds the source, session, input serial and observation time.
Active evidence expires. Candidate-window proximity is diagnostic only and cannot
authorize Enter. Composition finalization invalidates the query until the target
publishes a coherent fresh text/selection snapshot. Inputs lacking reliable
composition observations remain protected and may require F6; this is a limitation,
not successful IME support.

Verified standard Edit inputs additionally use a target-thread IMM observer when
TextEditPattern is unavailable. A read-only embedded DLL is installed with a
thread-specific WH_CALLWNDPROC hook. Its only operation reads composition text;
it does not intercept keys or change IME/editor state. The controller checks
process birth, control/thread identity, foreground focus, matching bitness, an
acknowledged request number and a bounded response time. A missing/failed observer
still means Unknown. Candidate-window visibility never authorizes Enter.
The DLL is built with the adapter and embedded in the executable. Its cache is
content-addressed, verified against the embedded bytes and held without write or
delete sharing. The hook and ephemeral mapping are retired with the target.
This adapter initially applies only to verified standard Edit controls; it is
not evidence of equivalent IME support in Chromium, RichEdit or another provider.

An active, verified IMM preedit becomes a temporary query through
`QueryRange::preview_composition`. This projection preserves the committed span
and revision, rejects context escape and invalid/oversized text, and cannot be
used for replacement. The presentation may show preview matches while keeping
all result execution disabled. After IME confirmation, a fresh committed
snapshot supplies the executable query. Tests of synthetic observations remain
distinct from Doubao/Microsoft Pinyin physical-input acceptance.

Exact replacement preflights the retained original text, proves the editor's
selection by readback, rechecks the ticket and clipboard, performs one native edit,
then verifies preserved context. Verified standard Edit controls use `EM_REPLACESEL`
with the frozen original text and Undo enabled. This closes the clipboard-reader
race where `WM_PASTE` can remove a selection without inserting clipboard text.
RichEdit and UIA editors retain native clipboard paste and original formats; an
unknown UIA editor never falls back to the standard Edit operation.
Chromium Edit providers may change paragraph separators when pasting rich HTML.
For a payload containing line breaks, receipt verification permits CR/LF changes
only inside the sealed insertion span; every other UTF-16 unit and the frozen
prefix/suffix must match. Single-line Chromium fields may also replace each line
break with a space. This changes readback expectations only, never the retained
clipboard formats, paste count, target identity checks, or request deadline.
After dispatch, UIA receipt reads the bounded document of the same focused editor
without reconstructing its caret subranges. Chromium may report inconsistent
selection subranges after multiline paste; those are still mandatory before
dispatch, but cannot veto an otherwise verified text receipt after dispatch.
An unknown delivery outcome blocks another replacement in
that session, retains focus and Enter protection, and requires the user to inspect
the input and explicitly cancel or switch modes. No Enter replay, whole-editor
overwrite or counted-backspace fallback is permitted.
Readback can use the remaining 900 ms request budget after an acknowledged edit;
there is no separate earlier 300 ms cutoff. This only retries observation, does
not extend the request deadline and does not repeat an uncertain mutation.

A current, verified nonempty selection serves as the frozen replacement range,
but does not seed search when Alt+V opens. Only subsequent entered text becomes
the search query; unchanged provider notifications keep the initial query empty.
Enter without typing can still replace that original selection with the chosen
item, and cancellation leaves it untouched.
Some Chromium providers return shifted `FindText` endpoints after
paragraph separators. When that search cannot prove the exact range, the adapter
locates provider Character boundaries with bounded measurements against the
frozen UTF-16 prefixes, then verifies the query and suffix before selection.
It does not equate Character units with UTF-16 units or split an unrepresented
grapheme boundary. Selection readback and the original deadline still apply.
An interior empty ProseMirror paragraph can collapse into a preceding separator
in UIA and create a new separator when typing begins. That unproven mapping is
explicitly unavailable and retains Enter protection; F6 remains available. A sole
empty paragraph and proven multiline ranges have separate supported paths.

`TargetCapabilities` distinguishes advertised selection from an exact selection
actually verified in the current session. The bounded `InlineTrace` ring contains
timing, event kinds and generations, never key text or composer/clipboard content.

## Rendering and search

Requested queries and displayed rows have separate readiness. Pending queries keep
the last complete rows and highlights; actions remain gated. A surviving selection
key remains selected when the new result arrives. Action accessibility reports
execution blocking without changing the pending state's icon opacity. A true empty
result replaces the old rows normally.

Nucleo searches complete space metadata with Unicode-whitespace normalization only
on the search side. Original UTF-16 query boundaries remain unchanged. Fuzzy cursors
bind space/revision/query. The cache retains bounded corpora and at most one small
normalized-query result page. `fuzzy_list_space_cancellable` checks cancellation
between metadata pages and bounded item batches; cancelled scans never return a
partial page as success. A newer list request or shutdown cancels old scans.
Every accepted request still produces a completion so cancellation cannot strand
the visible loading state or a pending side-card preview.

## Validation boundaries

Run the canonical `echo.cmd self-check`, `format --check` and `verify` gates.
`tests/native/Invoke-InlineAcceptance.py --help` lists the actual fixture runner
options. It holds a Windows foreground mutex and uses capture-disabled synthetic
data. `EchoTargetProbe.cs` is a separate, read-only HWND/PID capability probe;
advertised patterns do not certify replacement or IME support.
The `echo-windows` example `inline_target_probe` requires `native-test` and an
explicit foreground HWND plus expected synthetic Value. It uses the same COM
adapter as Echo and reports range lengths and identities without reading other
editors. Older managed UIA wrappers can omit newer advertised patterns, so their
pattern list is not evidence that TextEdit is absent.

An ordinary Electron window can receive only the Alt transitions because
`RegisterHotKey` removes the registered letter from its input stream. Its menu
then takes focus on Alt-up. The Windows hotkey adapter sends a tagged non-text
Control pair while the acknowledged Alt shortcut is still held, before
capturing the target. It preserves the real modifier release and does not type
text. It does not alter physically held Control or form Ctrl+Shift/Windows chords.
Acceptance must include separately paced down/up transitions, because a
single atomic SendInput chord can conceal this focus failure.

Codex's ProseMirror empty-paragraph decoration exposes a leading newline followed
by the accessible placeholder name through Chromium UIA. The adapter recognizes
it only with a collapsed caret at the start, a ProseMirror editor, one raw-tree
paragraph carrying the `placeholder` class, matching enclosing identity, and
matching TextPattern/Value text. Other paragraphs, literal label text, and real
newlines retain their original ranges. The controlled browser suite covers first
input, deleting back to empty, retyping, and preservation of literal label text.
ChatGPT's observed empty editor can instead expose just the generated label,
whose text differs from the editor's accessible name. That representation requires
a sole placeholder paragraph, a sole anonymous CSS group, and its sole text leaf.
The document must enclose that leaf, its name must equal the complete Value and
TextPattern text, and the caret must be collapsed at the document start. A real
text sibling, another paragraph, or a wider range prevents this normalization.
The leaf-decoration fixture hides its empty `br` to reproduce that native tree;
it also tests a real literal text node alongside the generated decoration.
The observed ChatGPT provider can flatten the anonymous CSS group as well. That
direct-leaf variant additionally requires its verified `prompt-textarea` identity
and `Chat with ChatGPT` accessible name; it is not enabled for an arbitrary
ProseMirror paragraph carrying literal text. Unrecognized variants retain their
text and protected failure behavior.

Chromium can return a FindText match whose paragraph offsets differ from the
current selection. A frozen preselection is reused only after an exact snapshot
comparison. Other matches fall back to bounded endpoint movement within the
verified editor document, measuring every prefix against the original UTF-16
text rather than treating UIA Character units as UTF-16 offsets. Selection is
still read back before the single native paste. An interior empty ProseMirror
paragraph whose caret shares the preceding paragraph boundary is explicitly
unsupported: Echo keeps Enter protected and offers F6 without merging paragraphs.
When a failed preflight positively identifies a different editor, the old lease
is cancelled; provider errors alone do not prove that focus moved.
`--record-screen` adds full-window physical screen recordings and records the
observed rate; `--stress` runs explicit minimum-count native loops. Browser fixtures
use normal accessibility activation by default; `--force-browser-accessibility`
is an explicitly labelled diagnostic option and never ordinary-application proof.
The test bridge retries response publication if a Windows reader temporarily
prevents atomic file replacement, without executing the request again.
Native acceptance can delay a single worker query, fail it, or publish its
original result after a newer result. These bounded controls and their counters
exist only with `native-test`, behind the capture-disabled fixture bridge.
The runner verifies that a fault actually occurred before claiming the scenario
passed, and records its own source plus fixture/driver hashes separately from
the tested application binary.
The adapter can delay acquisition after the actual hook acknowledges arming.
The test requires the Enter timestamp to lie between the recorded fault start
and end. A host-wide WM_GETTEXTLENGTH delay cannot prove this ordering because
another UIA client may consume it first. Additional isolated faults cover a
failed/no-op selection, selection readback beyond the request deadline, and an
unqueryable composition interface. Thumbnail tests hold one real response with
its original epoch/hash and release it after a new query. None of these controls
or response buffers are included in normal Release builds.
The request deadline atomically separates selection from native-write dispatch.
A timeout that wins before dispatch cancels the write and reports selection-only
uncertainty; a write that won first retains replacement-unknown protection.
An eventual selection reply cannot cross that cancelled dispatch boundary.
The standard Edit fault test acquires the clipboard from a second thread only
after the selected-range operation arrives. It verifies preserved text and one
Undo while that reader is holding the clipboard. Delayed native edit replies also
exercise the unknown-outcome protection without replaying the mutation.
Pointer tests bind down/up to the verified physical virtual-desktop coordinates,
check the non-activating style, and record foreground identity before/after input.

Fixture, ordinary-application, physical-keyboard and Release evidence are separate.
Inline result height changes reflow the visible cards inside a stable native
canvas sized for the available side of the input. The visible card edge stays
anchored to the caret. Main-card bounds, side-texture layout and pointer regions
use the same card origin. This avoids resizing the HWND while Windows still
displays the previous GPU buffer. The transparent unused canvas is outside the
native card region. Renderer and physical-screen evidence must both be checked:
row counts alone cannot detect an old frame cropped by a native resize.
Changing the native region can also crop an older displayed buffer. Inline
updates first union the previous and requested regions. A generation-tagged
command queued from winit's propagated redraw commits the smaller region after
draw/present and DWM synchronization. Older queued generations cannot shrink a
newer requested region. This path applies to GPU and software renderers.
The virtual ListView's viewport can briefly be zero after a model update.
Before measuring it, native composition runs the same bounded tree-instantiation
pass used by Slint 1.17.1's renderer and accessibility adapter. This pinned
internal-runtime seam is confined to `quick_insert_window.rs`, runs on the UI
thread, and performs no rendering or GPU readback. Runtime upgrades must retain
the zero-result-to-first-row screen regression before changing this seam.
Presentation retains the query represented by a completed snapshot even when
that snapshot contains zero rows. Pending or failed requests do not erase that
identity or replace its empty-state message with a loading placeholder. Actions
continue to require the current request's executable state.

Never inject into the Codex task implementing the change. Test real applications
in dedicated empty drafts without external Enter interception or forced
accessibility flags. Report each required case with its exact binary and source
identity; an unrun or failed mandatory case prevents final acceptance.
`--application-target` accepts exactly one explicit HWND/process-birth/image
registration, with executing-window exclusions and `cleanup_process: false`.
Every input verifies the dedicated draft marker, focused editor and allowed
synthetic values again. A vanished draft or unexpected text stops the run. The
ordinary application suite performs a local replacement and one Undo, then clears
only the verified synthetic query. It never tests an unguarded post-close Enter in
a real chat or task composer. The registration is never used to close the shared
application process.

Chinese composition search previews ignore ASCII apostrophes between ASCII
letters only inside verified active preedit (`e'ch` searches as `ech`). UIA
TextEdit previews retain the active range's UTF-16 span and document and require
an exact match with the current snapshot. Native EDIT previews project the
target-thread preedit at the snapshot selection. Both paths leave the original
query, replacement span and revision untouched. Committed text, other input
languages and text outside the composition keep their apostrophes. Missing or
inconsistent composition ranges use the existing protected fallback; they never
trigger whole-query punctuation removal. Filtering and highlighting consume the
same projected query, while candidate keys remain with the input method.
For Chromium rich editors, freeze the active range's text before inspecting its
enclosing element or endpoints: those calls can normalize the provider range to
its first text leaf. Verify editor ancestry, measure the start, and require the
entire frozen preedit to equal the current document slice. Never substitute a
later, truncated GetText result or infer a range by searching for similar text.
Regression review must include an initially empty contenteditable paragraph:
activate Echo, type `w`, `h`, `e` with Chinese IME and wait at `w'he`; `when`
must remain matched. Repeat with `e'ch`, Backspace and composition cancellation.

For UIA editors, the session-owned target-thread observer also reads the existing
TSF thread manager's focused context and enumerates its active compositions.
Chromium may retain a nonempty UIA composition range after cancellation or
confirmation, even across Echo sessions; a successful live TSF read owns lifecycle
while UIA still supplies verified preview text. No TSF manager is created or
activated and no edit session or candidate operation is requested. Unsupported
observation retains the UIA fallback; a failed previously established observer
read is Unknown and cannot authorize Enter. The ephemeral observer protocol is
version 2; stored content and clipboard representations do not change.
Activation schedules a fresh observation after subscriptions are installed.
Late composition events cannot overwrite a newer target read or another session.
Native regression review must cover Esc cancellation, Return confirming English,
and Space confirming Chinese: Up/Down and Tab/Shift+Tab resume in the same popup
and after reopening, without moving source focus or submitting the fixture.
