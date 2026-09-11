# Legacy native UI assertion retirement map

The removed `Invoke-UiAcceptance.ps1` was a noncanonical inventory. Current
commands keep their existing names and use synthetic fixtures and isolated data.
Historical source remains in Git at `2930ad4` and earlier commits.

| Legacy assertion | Current owner / disposition |
| --- | --- |
| semantic-startup-one-window | `Invoke-Smoke.ps1` startup identity and one-window checks |
| smoke-manual-history, close-to-hide, graceful-exit | Current read-only smoke; public History activation, resident hide, explicit Quit |
| copy-roundtrip | Inline `manual-history-row-copy-retains-text` and clipboard original-format roundtrip |
| search | Inline multiword/scoped/streaming query checks; storage large-corpus gate |
| history-favorite-create-edit-delete | Software `saved-content-create-edit-icon-delete`, including persisted name/content and case-insensitive Mail icon selection; the old tag-entry control is no longer exposed, while retained tag data remains storage-owned |
| batch-and-clear-cancel | Software `history-clear-cancel-retains-content`; the old Select/All loaded/Cancel toolbar entry flow is no longer exposed. Preserved presentation tests `batch_does_not_invent_ids` and `panel_switch_cancels_batch_and_pending_load` cover retained state semantics |
| settings-invalid-valid-theme-about | Software `settings-invalid-valid-and-about` plus `settings-light-dark-and-software-preview`; zero rejected, supported 2,000 accepted. The old 5,001 setting is incompatible with the current 2,000-entry contract |
| image-thumbnail-hover | Software M mixed-image scroll, side-card previews and rendered composition; inline retained image insertion. Old tooltip/row label assumptions are retired |
| favorites-only-close | Not applicable: independent Favorites and main windows were retired in favor of one window |
| second-instance-handoff-and-malformed-envelope | Software `public-activation-cancels-old-carousel` and `activation-replay-and-malformed-envelope`; smoke activation restoration |
| activation-replay | Software `activation-replay-and-malformed-envelope`, repeated same request and one surviving resident |
| settings-and-favorite-persist-after-restart | Software `settings-and-deletion-persist-after-restart`; supported History limit, storage limits, and deleted favorite checked after a real process restart |
| safe-paste-owned-target | Inline exact replacement, original image insertion, prefix/suffix and selection boundary checks |
| paste-failure-restores-clipboard | Inline failure/cancellation and clipboard preservation wrapper; clipboard format gate checks retained original formats |

No UIA or synthetic input result certifies physical IME or multi-display behavior.
Those environments are excluded by the owner for this retirement round.

## Actual editor input

The existing application target harness now supports explicitly registered Edit
or Document controls. HWND, executable path, process birth, focused composer,
draft marker and exact allowed synthetic values remain mandatory. An optional
finite `window_titles` list accounts for a document's modified-title state;
there is no wildcard title matching. `paced_text` delivers one Unicode character
at a time and verifies the resulting prefix before proceeding. It is separate
from the unchanged fast-input tests in controlled native fixtures. Registration
never grants process cleanup ownership for a shared editor.

This registration was accepted for one dedicated file in an ordinary Notepad
window. It does not certify arbitrary multi-tab editors or distinguish two
same-named files within one shared window. Those targets require additional
active-document identity proof before using this harness.

## Retained batch code versus retired entry flow

`space-panel.slint` renders the batch toolbar only under `if (root.batch)`;
`entry-row.slint` likewise renders Toggle selection only under that condition.
The only production transition setting batch true is `App::batch_action` handling
`begin`. Current Slint consumers forward only all/favorite/pin/delete actions;
none emits begin. Keyboard ToggleBatch is itself guarded by `key.batch`.
Therefore these retained conditional controls do not establish a reachable
Select/All loaded/Cancel entry flow. Native initial-tree evidence confirms those
controls are absent. The conditional code and presentation state tests remain;
this retirement removes the obsolete test script, not the retained batch state.
