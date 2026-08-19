# Codex Prompt — Iris Extraction Agent

Workspace: `D:\Projects\iris`

Source reference: read-only access to `D:\Projects\culsans`.

Read the architecture pack and `docs/06_IRIS_PLAN.md`.

Mission:

Build Iris as a standalone capture/pin/drawing product.

Before editing:

- inventory capture, precision capture, editor, pin, Fabric, Excalidraw, drawing storage, drawing library, settings and tests;
- write `docs/MIGRATION_MAP.md`;
- record the Culsans baseline SHA.

Hard rules:

- no sibling source/path dependencies;
- visual dependencies belong to Iris;
- drawing persistence belongs to Iris;
- preserve behavior before redesign;
- implement activation v1;
- Iris must run independently of Culsans.

Definition of Done:

Iris builds/runs/tests without Culsans source and Culsans can delete capture/drawing implementation and visual-only dependencies.
