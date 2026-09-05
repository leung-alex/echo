> Historical record only. It describes the retired React/Tauri parity implementation and is not an active implementation contract. Current routing is in `docs/architecture/overview.md` and `docs/architecture/locality.md`; current visual source is `apps/desktop/ui`.

# Echo Visual Parity Deviations

Baseline: Culsans `e625306a8b23e3cc1326738e528612455f0a4db6`.

The retired Echo UI used Echo-owned React components, tokens, primitives, and CSS. The following historical differences were intentional and did not introduce a new product surface:

- Culsans `draftRecovery` remained a Culsans shell-owned setting and was not rendered or stored by Echo.
- Culsans backup/restore buttons were omitted because Echo had no user-facing backup/restore command; migration backup remained an internal safety operation.
- Echo retained its existing `record_sensitive` setting because it was part of the Echo clipboard API.
- Echo used Echo-local snake_case IPC payloads and types instead of Culsans contracts.
- The historical image preview command was superseded by the binary preview resource seam.
- The Echo settings route had Echo-local navigation and did not import Culsans shell navigation.

These React, CSS, TypeScript, and IPC details are historical evidence only. They must not be restored as dependencies of the native Rust/Slint application. The product distinctions they record—especially Saved Items, sensitive-content policy, and the absence of Culsans-only surfaces—remain applicable where reflected in current domain contracts.
