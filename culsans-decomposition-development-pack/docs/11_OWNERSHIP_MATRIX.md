# Ownership Migration Matrix

This matrix is the primary review checklist.

| Capability / asset | Current owner | Target owner | Culsans action |
|---|---|---|---|
| Command Panel | Culsans | Culsans | Keep |
| Input Agent | Culsans | Culsans | Keep |
| Gesture/Input | Culsans | Culsans | Keep |
| Browser integration | Culsans | Culsans | Keep |
| Shell settings | Culsans | Culsans | Keep |
| Window/focus shell context | Culsans | Culsans | Keep |
| File Search UI | Culsans | Aster | Delete from Culsans |
| File Search runtime/service | Culsans | Aster | Delete from Runtime |
| `culsans-search` File Search ownership | Culsans | Aster | Migrate/rename as needed |
| Everything executable | Culsans bundle | Aster | Remove from Culsans bundle |
| Everything config/lifecycle | Culsans | Aster | Remove |
| Search settings | Culsans | Aster | Remove/redirect |
| Search diagnostics | Culsans | Aster | Remove/redirect |
| Capture | Culsans | Iris | Remove |
| Precision capture | Culsans | Iris | Remove |
| Capture editor | Culsans | Iris | Remove |
| Pin | Culsans | Iris | Remove |
| Fabric | Culsans frontend | Iris | Remove dependency |
| Excalidraw | Culsans frontend | Iris | Remove dependency |
| Drawing documents | Culsans storage | Iris | Migrate |
| Drawing library | Culsans storage | Iris | Migrate |
| Drawing settings | Culsans | Iris | Remove/redirect |
| Clipboard listener | Culsans | Echo | Stop owning |
| Clipboard DB schema | Culsans storage | Echo | Migrate |
| Clipboard blobs | Culsans data | Echo | Migrate |
| History | Culsans | Echo | Remove |
| Favorites | Culsans | Echo | Remove |
| Saved insert items | Culsans | Echo | Remove |
| Snippets | Culsans | Echo | Remove |
| Quick Insert UI | Culsans | Echo | Remove |
| Paste/insertion workflow | Culsans runtime | Echo | Remove |
| Clipboard settings | Culsans | Echo | Remove/redirect |
| App registry | none/implicit | Culsans | Add |
| External activation encoding | none | Culsans + each app boundary | Add locally |
| App-private single instance IPC | mixed | each app | App-owned |
