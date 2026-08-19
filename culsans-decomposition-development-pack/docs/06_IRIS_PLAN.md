# Iris Extraction Plan

## Product statement

**Iris — See**

A standalone visual utility for capture, annotation, pinning, and drawing.

## Source ownership to migrate

Inventory and migrate:

- screenshot capture
- precision capture
- capture frame/result types used only by visual workflows
- capture editor
- pin
- annotation tools
- Fabric canvas logic
- Excalidraw drawing
- drawing documents
- drawing library
- thumbnails
- image export
- drawing persistence
- capture/drawing settings
- related Tauri commands
- related tests

The current `culsans-capture` crate is a natural ownership candidate for Iris.

The current frontend's Excalidraw/Fabric dependencies must leave Culsans unless some shell feature independently needs them.

## Repository bootstrap

`D:\Projects\iris`

Recommended high-level structure:

```text
iris\
├── apps\desktop\
├── backend\crates\iris-capture\
├── backend\crates\iris-storage\
├── frontend\app\
├── tests\
└── ...
```

## Extraction strategy

### I1 — Capture tracer bullet

First get:

```text
launch Iris capture
→ select/capture
→ show capture editor/result
→ close/save/copy
```

working independently.

### I2 — Pin workflow

Move pin windows and pin lifecycle.

Iris owns pin window placement/state.

### I3 — Drawing workflow

Move:

- Excalidraw
- drawing document model
- drawing library
- thumbnails
- open/save/rename/delete
- search within Iris if applicable

Do not preserve a Culsans drawing repository API.

### I4 — Visual persistence

Create Iris-owned persistence.

Target:

```text
%LOCALAPPDATA%\Iris\
    iris.sqlite3
    ...
```

Drawing tables migrate out of Culsans.

### I5 — Settings

Move capture/drawing/pin settings into Iris.

### I6 — Activation

Implement:

- `iris.open`
- `iris.capture`
- `iris.capture_precision`
- `iris.drawing`
- `iris.settings`

### I7 — Packaging

Iris has:

- independent Tauri identifier
- own installer
- own icon/product metadata
- own version
- no dependency on Culsans source

## Behavioral preservation

During extraction, do not simultaneously redesign:

- capture gestures
- drawing tool semantics
- pin interaction
- editor layout

First establish independent ownership. Redesign later in Iris.

## Local tests

Required:

- capture region
- cancel capture
- precision capture
- capture editor open/close
- copy/save result
- pin lifecycle
- drawing CRUD
- drawing library persistence
- corrupt/oversized drawing handling
- activation while running/not running
- settings persistence
- independent build without Culsans

## Exit condition

Iris is complete only when Culsans can remove capture/drawing business code, visual persistence, and Iris-only frontend dependencies.
