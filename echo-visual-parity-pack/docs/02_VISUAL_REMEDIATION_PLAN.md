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
