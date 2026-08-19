# Echo P06 — Test Ownership Extraction

## Legacy Culsans assets to inspect

Primary candidates:

```text
tests/e2e/clipboard-first-open.spec.ts
tests/e2e/clipboard-system.spec.ts
tests/e2e/clipboard-visual.spec.ts
tests/e2e/quick-insert.spec.ts
```

Inspect imported helpers/fixtures and move only Echo-owned parts.

## Required confidence areas

- first open,
- listener active while UI hidden,
- text clipboard,
- HTML/RTF behavior,
- image clipboard,
- file-list clipboard,
- dedup/fingerprint,
- sensitive-source policy,
- blob storage,
- retention/GC/reconciliation,
- History,
- Favorites,
- Snippets,
- Quick Insert list/search,
- Copy,
- Insert/paste,
- invalid/closed paste targets,
- hide/reopen,
- single-instance behavior,
- legacy-data migration,
- packaging smoke.

## Boundary

Culsans Input Editor tests remain in Culsans if they prove Input Editor/shell behavior.

Echo owns only the receiving/persistence side of any snippet handoff.

## Frontend tests

The current frontend `test` script is effectively typechecking. Add real component and/or Playwright UI coverage so the destination confidence level is not weaker than the old Culsans tests.

Create `docs/TEST_OWNERSHIP_MAP.md`.
