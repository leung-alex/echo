# E07 — Go Repository Tooling

## Objective

Create the independent `Echo` repository tool described in `docs/07_GO_REPOSITORY_TOOLING.md`.

## Required work

- root `.cmd` entry,
- repo-local Go module,
- hashed bootstrap cache,
- per-run isolation,
- install/format/verify/self-check/build/dev/smoke/acceptance/package/release-candidate/sync/clean,
- changed-owner verification,
- product-specific owner groups,
- Go self-tests,
- update README with normal developer workflow.

## Exit criteria

- public root command is sufficient for ordinary repository development,
- Go tool tests/vet pass,
- product acceptance is wired to P06 tests,
- no sibling source dependency exists.
