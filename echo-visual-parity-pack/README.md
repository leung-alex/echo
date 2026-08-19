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
