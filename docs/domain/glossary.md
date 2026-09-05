# Echo Domain Glossary

- **Clipboard Item**: one captured clipboard state with source metadata and one or more retained original representations.
- **Representation**: one format/MIME/byte payload belonging to a Clipboard Item, such as text, HTML, RTF, image, or files.
- **History Entry**: the persisted searchable timeline record for a Clipboard Item. History is not the durable reusable-content model.
- **Original Payload**: the complete persisted set of representations used for copy and insert, not preview text, a thumbnail, or Slint model data.
- **Preview Asset**: a bounded visual or textual derivative used for display instead of loading the Original Payload into the UI.
- **Thumbnail**: a bounded image Preview Asset generated from original content and stored by content identity.
- **Favorite**: the user-facing label for a Saved Item in Echo's UI.
- **Saved Item**: a distinct durable snapshot of a clipboard payload that remains after its source History Entry is cleared or deleted. User edits can make a linked snapshot independent.
- **Quick Insert**: the retrieval and insertion surface; it is a use case, not a stored data type.
- **Quick Insert Session**: the engine/presentation-coordinated session containing the safe paste target captured before Echo is shown and an epoch used to reject stale work.
- **Paste Target**: the native input window/control and process instance that may receive a paste only after revalidation.
- **Surface**: framework-independent presentation state for one History or Favorites result view, including query, pagination, selection, and status.
- **Row Key**: an opaque string created and resolved by `echo-presentation` to identify a loaded row across the Slint callback boundary. It is not a public serialization of a database ID.
- **Worker Message**: a typed Rust work request or completion exchanged between desktop UI-thread orchestration and background services.
- **Activation Envelope**: a bounded, versioned Echo request passed with the canonical `--echo-activate` flag.
- **Resident Host**: the primary Echo process for one Windows user, logon session, and canonical data directory. Secondary invocations forward activation to it.
- **Close to Hide**: the lifetime rule that dismissing or closing Echo hides its windows while capture remains resident.
- **Quit**: the explicit action that stops capture and shuts down the event loop, worker, storage runtime, named pipe, and tray host.
