# Echo Visual Parity Deviations

Baseline: Culsans `e625306a8b23e3cc1326738e528612455f0a4db6`.

Echo uses Echo-owned React components, tokens, primitives, and CSS. The following differences are intentional and do not introduce a new product surface:

- Culsans `draftRecovery` remains a Culsans shell-owned setting and is not rendered or stored by Echo.
- Culsans backup/restore buttons are omitted because Echo has no user-facing backup/restore command; migration backup remains an internal safety operation.
- Echo retains its existing `record_sensitive` setting because it is part of the Echo clipboard API.
- Echo uses Echo-local snake_case IPC payloads and types instead of Culsans contracts.
- Image previews use the narrow Echo-owned `quick_insert_get_image` read command over existing stored representations; no storage schema is changed.
- The Echo settings route has an Echo-local back control and activation wiring; it does not import Culsans shell navigation.
