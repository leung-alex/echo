# P07 Shared Go Repository Tooling Requirements

## Goal

Recreate the **developer-experience class** of Culsans `cu.cmd + tools/cu`, but as a completely independent repository-local tool.

Do **not** share Go source between products.

## Bootstrap

Create a root `.cmd` public entry and a repo-local Go module. Keep the useful Culsans bootstrap pattern:

1. locate repository root,
2. validate a pinned Go version (default to the same `go1.26.2` baseline used by Culsans unless the repository has a documented reason to change it),
3. hash Go tool source inputs,
4. compile/cache the tool by source hash,
5. copy the cached executable into a per-run isolated session,
6. forward arguments unchanged,
7. clean the transient session executable.

The compiled repository tool is an implementation detail; the root `.cmd` is the public entry.

## Minimum public commands

- `help`
- `install`
- `format [--check]`
- `verify [--changed-from <sha>] [--profile <developer|ci>] [--explain]`
- `self-check`
- `build [--release]`
- `dev`
- `smoke`
- `acceptance <owner>`
- `package [--dir]`
- `release-candidate`
- `sync [--all | --branch <name>]`
- `clean`

Add `benchmark` only where the product already has a meaningful benchmark.

## Changed-owner verification

`verify --changed-from` must plan affected owners from Git changes, then run only relevant gates.

It must support:
- deterministic owner mapping,
- `--explain`,
- developer vs CI profile,
- validation that unknown/ambiguous changed paths fail safely rather than silently skipping coverage.

## Self-check

At minimum verify:
- root command/bootstrap integrity,
- Go tool tests,
- local-only paths,
- no sibling path dependency,
- expected manifests/scripts,
- owner graph consistency,
- generated-file drift if applicable,
- package/installer inputs,
- P06 test ownership map completeness.

## Tool self-tests

The Go tool itself must pass:

```text
go test ./...
go vet ./...
```

Add tests for dispatch, flags, repo-root resolution, changed-owner planning, command construction, independence guards, and no sibling mutation.



# Echo-specific P07 Requirements

Public entry:

```text
echo.cmd
```

Go tool root:

```text
tools/echo/
```

Required focused commands:

```text
echo.cmd verify clipboard
echo.cmd verify quick-insert
echo.cmd acceptance clipboard
echo.cmd acceptance quick-insert
```

Recommended owners:

```text
clipboard
storage
library
quick-insert
desktop
frontend
migration
installer
tests
tooling
```

`benchmark` is optional in P07 unless a meaningful existing benchmark is migrated.

Do not copy unrelated Culsans Browser/Input/Capture/Everything tooling.
