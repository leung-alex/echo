# External Application Activation Contract v1

## Purpose

Provide the smallest stable seam between Culsans and the three independent applications.

The first version is intentionally **one-way**:

```text
Culsans -> launch/activate external app
```

No shared runtime SDK is introduced.

## Transport

Culsans launches the target executable with a semantic activation envelope.

Recommended CLI form:

```text
<app>.exe --culsans-activate <BASE64URL_UTF8_JSON>
```

Base64url avoids Windows quoting problems for JSON payloads.

Each app must:

1. decode the envelope,
2. validate `version`,
3. validate the action/payload,
4. become or locate its single primary instance,
5. forward the activation to its own primary instance if necessary,
6. exit the secondary bootstrap process.

The app-private single-instance IPC is owned entirely by that app and is not part of the Culsans protocol.

## Envelope

Conceptual shape:

```json
{
  "version": 1,
  "request_id": "uuid",
  "action": "open",
  "origin": {
    "platform": "windows",
    "foreground_hwnd": "optional opaque string",
    "foreground_pid": 1234
  },
  "payload": {}
}
```

### Rules

- Unknown envelope versions must fail safely.
- Unknown actions must fail safely.
- `request_id` is for diagnostics; it does not imply a callback.
- `origin` is optional context, not authority.
- External apps must validate any supplied window handle before use.
- Payloads must remain small.
- Never put binary screenshot/clipboard data in this envelope.
- Never put shared database paths in this envelope.

## Action set v1

### Aster

```text
aster.open
aster.search
aster.settings
```

Suggested payload for `aster.search`:

```json
{ "query": "optional initial query" }
```

Aster decides filters, scope defaults, backend, and result behavior.

### Iris

```text
iris.open
iris.capture
iris.capture_precision
iris.drawing
iris.settings
```

Capture/drawing internals remain private to Iris.

### Echo

```text
echo.open
echo.quick_insert
echo.settings
```

Echo owns target validation and paste execution.

## App discovery

Production target:

- each product installer registers its executable through a standard Windows application discovery mechanism such as App Paths,
- Culsans resolves the executable without hard-coding a sibling source path.

Development target:

- Culsans may provide explicit developer executable overrides in shell-only developer settings/environment variables.

Forbidden:

```text
D:\Projects\aster\target\debug\aster.exe
```

hard-coded into production source.

## Compatibility policy

v1 is tiny so that apps can evolve independently.

Compatibility rule:

- adding an optional payload field is backward compatible,
- adding a new action is backward compatible,
- changing semantics of an existing action is breaking,
- removing an action is breaking,
- changing the envelope shape incompatibly requires version 2.

Do not synchronize product versions.

Example valid state:

```text
Culsans 0.8
Aster   0.14
Iris    0.6
Echo    0.11
```

## Explicit non-goals

v1 does not provide:

- remote procedure calls into app internals
- live state mirroring
- cross-app shared settings
- a common event bus
- callbacks to Culsans
- shared Rust/TypeScript protocol packages

If a callback is later required, design it as a separate capability after concrete use cases exist.
