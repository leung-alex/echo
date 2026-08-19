# EXECUTE — Echo P08 Presentation Layer Restoration

This is an extraction-defect correction.

Do not keep the current string-rendered `innerHTML` UI as final presentation. Use Culsans `main@e625306a` as read-only source of truth for the original Clipboard / Quick Insert UI.

Restore the React presentation architecture, QuickInsertSurface/controller/components/model, clipboard/quick-insert CSS, required UI primitives, localized Echo design tokens and Clipboard Settings presentation.

Keep the extracted Echo backend, storage, clipboard engine, library and Quick Insert service. This is presentation restoration plus API rewiring, not backend redevelopment.

Do not modify Culsans and do not introduce sibling source dependencies.

Produce `docs/VISUAL_PARITY_DEVIATIONS.md`, rendered evidence and `docs/P08_VISUAL_HANDOFF.md`.
