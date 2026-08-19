# Echo — P08 Visual Parity Remediation Master Plan


---

# Echo P08 — Presentation Layer Restoration

Echo backend/domain extraction is valid, but its frontend presentation layer was not actually migrated. The current UI is a string-rendered `innerHTML` implementation with a new light/teal/orange visual language.

That is not extraction parity.

P08 restores the original Clipboard / Quick Insert presentation architecture into Echo while keeping Echo backend ownership intact.


# Locked Rules — Visual Parity Remediation

1. This phase is an **extraction defect correction**, not a redesign.
2. Culsans `main@e625306a` remains the visual source of truth for the extracted surfaces until parity is proven.
3. Do not introduce a runtime/source dependency on Culsans.
4. Do not create a shared UI package spanning repositories.
5. Localize required design tokens and UI primitives into the destination repository.
6. Preserve interaction semantics, dimensions, spacing, typography, colors, states, focus behavior and window behavior unless an intentional deviation is explicitly documented.
7. Do not change backend/business behavior except where needed to wire the restored UI to destination-owned APIs.
8. Do not add new product features.
9. Build/typecheck/unit tests are insufficient by themselves; visual parity requires rendered evidence.
10. Culsans must not delete corresponding legacy frontend/CSS source until the destination handoff is delivered.
11. Prefer mechanical localization/renaming over creative restyling.
12. Every intentional visual deviation must be listed in `docs/VISUAL_PARITY_DEVIATIONS.md`.


---

# Echo Visual Audit

## Culsans source-of-truth presentation

```text
frontend/app/src/features/quick-insert/QuickInsertRoute.tsx
frontend/app/src/features/quick-insert/QuickInsertSurface.tsx
frontend/app/src/features/quick-insert/controller.ts
frontend/app/src/features/quick-insert/components/
frontend/app/src/features/quick-insert/model/
frontend/app/src/features/clipboard/clipboard.css
frontend/app/src/features/clipboard/clipboard-settings.css
frontend/app/src/features/clipboard/ClipboardSettingsPage.tsx
```

Required shared primitives include `SearchField`, `SearchMatchText`, `useActiveResultNavigation`, and any Switch/Badge controls actually used.

## Current defect

Echo currently:
- does not preserve the React Quick Insert presentation,
- uses `app.innerHTML` for the major UI,
- owns a new light/teal/orange theme,
- therefore requires presentation restoration, not cosmetic CSS patching.

## Correct model

Migrate:

```text
presentation behavior
+ feature CSS
+ required UI primitives
+ design tokens
```

then rewire API calls to Echo-owned commands/services.


---

# Echo P08 Presentation Restoration Plan

## E08.1 — Restore React presentation stack

Bring back a React frontend architecture for Quick Insert/Library surfaces. Add repo-local React/ReactDOM/icon dependencies actually required by the preserved UI. Do not add `@culsans/ui` or `@culsans/contracts` sibling dependencies.

## E08.2 — Port original Quick Insert presentation

Port/adapt:

```text
QuickInsertRoute.tsx
QuickInsertSurface.tsx
controller.ts
navigation.ts
components/
model/
```

Keep Echo backend/service ownership and rewire the API adapter to Echo commands.

Preserve history/favorites/snippets tabs, search, detailed/compact views, keyboard navigation, selection, copy/insert, empty/error states, target-safe behavior and focus/close semantics.

## E08.3 — Restore feature CSS and Echo token layer

Create:

```text
frontend/app/src/echo-ui.css
frontend/app/src/ui/
```

Port the relevant `clipboard.css` and `clipboard-settings.css` and mechanically localize:

```text
--culsans-* -> --echo-*
.culsans-*  -> .echo-*
```

Localize only required primitives: likely SearchField, SearchMatchText, useActiveResultNavigation, Switch, Badge.

## E08.4 — Restore Settings presentation

Port/adapt the original Clipboard Settings structure while preserving Echo-owned settings API/storage.

## E08.5 — Remove temporary UI

After parity is green, remove the current string-rendered presentation and unrelated temporary palette. Do not keep two competing presentation systems.

## Required rendered states

Quick Insert: History detailed, History compact, Favorites, Snippets, active search, selected result, empty, copy, insert, invalid target/error.
Settings: default, toggles, limits, focus/hover, save/error.


---

# Echo P08 Acceptance

Complete when:
- Quick Insert presentation is React/component-based again,
- original interaction structure is restored,
- original visual language is restored using Echo-local tokens,
- temporary `innerHTML` presentation is removed,
- no Culsans source/UI package dependency exists,
- no unresolved `--echo-*` token remains,
- settings visually match the restored language,
- screenshot evidence covers the required state matrix,
- Echo backend/domain tests remain green.

Run:

```text
.\echo.cmd verify
.\echo.cmd acceptance clipboard
.\echo.cmd acceptance quick-insert
.\echo.cmd package
```

If E07 is incomplete, use equivalent local commands and record them.
