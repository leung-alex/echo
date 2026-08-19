# Echo Engineering Ownership Acceptance

## Completion definition

`Echo` is engineering-owned only when:

1. product-owned unit/component/integration/system/performance tests live in this repository,
2. those tests launch `Echo` directly,
3. no required product confidence still depends on Culsans test harnesses,
4. the repository has its own Go orchestration entry,
5. the Go tool can install, verify, build, run, smoke-test, acceptance-test and package `Echo`,
6. changed-owner verification is functional,
7. tool self-tests pass,
8. sibling repositories are unnecessary to build/test/package.

## Handoff artifact

Create:

```text
docs/P06_P07_HANDOFF.md
```

It must list:

- source baseline inspected,
- test ownership map,
- migrated/rewritten/replaced tests,
- tests intentionally staying in Culsans,
- new root command surface,
- Go owner graph,
- executed gates and results,
- Culsans legacy tests now safe to delete,
- Culsans `cu` product-specific gates now safe to delete,
- remaining blockers,
- final commit SHA.

Do not delete Culsans files in this agent.
