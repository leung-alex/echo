# Echo Domain Glossary

- **Clipboard Item**: one captured clipboard state with source metadata and
  one or more representations.
- **Representation**: one format/mime/bytes payload belonging to a Clipboard
  Item, such as text, HTML, RTF, image, or files.
- **History Entry**: the persisted searchable record for a Clipboard Item.
- **Original Payload**: the complete persisted representations used for copy
  and insert, not the text preview.
- **Preview Asset**: a bounded visual or textual representation used for UI
  display instead of the Original Payload.
- **Thumbnail**: a bounded image Preview Asset generated from the original
  payload and stored by content identity.
- **Favorite**: the user-facing label for a Saved Item in History and Quick
  Insert.
- **Saved Item**: a durable snapshot of a clipboard payload that remains after
  its source History Entry is cleared or deleted.
- **Quick Insert Session**: the engine-owned session containing the safe paste
  target captured before Echo is shown.
- **Paste Target**: the native input window/control and process instance that
  may receive a paste after revalidation.
