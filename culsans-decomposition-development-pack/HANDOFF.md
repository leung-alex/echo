# Handoff to Fresh Codex Sessions

The user has approved a hard split of the current Culsans product into four fully independent sibling projects:

```text
D:\Projects\culsans
D:\Projects\aster
D:\Projects\iris
D:\Projects\echo
```

The three extracted products are:

- Aster — File Search / Find
- Iris — Capture + Pin + Drawing / See
- Echo — Clipboard + History + Favorites + Snippets + Quick Insert / Recall

The extraction and Culsans runtime contraction must proceed in parallel and cut over together.

Do not re-open the already-resolved monorepo vs multi-repo decision.

Primary docs:

- `README.md`
- `docs/01_TARGET_ARCHITECTURE.md`
- `docs/02_PARALLEL_EXECUTION.md`
- `docs/03_EXTERNAL_APP_PROTOCOL_V1.md`
- repository-specific plan
- `docs/09_TEST_AND_ACCEPTANCE.md`

Suggested skills for an agent where available:

- engineering-workflow-guide
- to-spec
- to-tickets
- implement
- code-review
- diagnosing-bugs
- handoff

Critical architecture rule:

**No source-level dependency across the four sibling repositories.**

The goal is ownership migration, not file relocation.
