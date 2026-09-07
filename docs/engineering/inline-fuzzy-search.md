# Inline fuzzy search and editor lifetime

## Behavior

Alt+V starts an ephemeral range in the original input. Text remains in that input; Echo renders suggestions without activating its window. Space is part of the query and never commits or dismisses the session. Unicode whitespace, including contenteditable NBSP, separates fuzzy query words. All words must match, in any word order; each word uses nucleo's subsequence matching. Empty queries preserve chronological or manual ordering.

Search ranks the entire selected space, not only the visible page and not an FTS-prefiltered subset. Metadata is paged through the existing engine store contract. At most three corpora totaling 16 MiB are cached; oversized corpora are streamed rather than silently truncated. Ranked result pages are bounded. Cursors are bound to space, revision and normalized query. Hiding the session releases cached corpora.

## Text and safety

UIA TextPattern is reacquired from the same pinned editor on every observation. Replacing an empty paragraph must not leave Echo attached to an obsolete paragraph provider. A collapsed caret at the start of a sole empty HTML paragraph may have a synthetic terminal newline; selected newlines, multiple blank lines, native Edit contents and nonempty text are not broadly trimmed.

The engine's replacement span keeps the original prefix and suffix immutable. Query whitespace is normalized only for matching, never by rewriting the input. Exact selection and current text are verified before paste, with bounded acknowledgement waiting. Replacement uses retained original clipboard representations. Rendered markup is never an insertion payload.

## Stable UI

A pending query keeps the last complete rows, selected appearance and action strip visible. Execution readiness is independent from button opacity; stale actions are ignored. Row updates are keyed and unchanged values do not emit model changes. Empty-state/height changes occur when current results are ready, not at each key-down.

Matched graphemes are bold and amber-toned in title, body and tags. Markup punctuation from stored text is escaped. Accessibility labels remain plain text. Match indices are grapheme indices converted to UTF-8 byte ranges, not UTF-16 offsets. Side-card captures use the same highlighting.

## Regression evidence

Run the engine/desktop/storage tests and the opt-in `tests/native/Invoke-InlineAcceptance.py` against capture-disabled synthetic data. Cases include an empty rich paragraph, first-character provider replacement, NBSP/multiple spaces, top omnibox, Unicode, held Enter, exact prefix/suffix preservation and physical pixel samples of the action strip. Tests do not certify unexecuted physical IME or arbitrary third-party editor implementations.
