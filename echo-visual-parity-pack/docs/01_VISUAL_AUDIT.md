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
