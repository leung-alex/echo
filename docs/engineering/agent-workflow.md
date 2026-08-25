# Agent Workflow

1. Confirm the worktree, branch, base SHA, and owned paths before editing.
2. Read `AGENTS.md`, the architecture overview, dependency rules, and the
   test ownership map for the affected seam.
3. Make the smallest coherent change in the owning module. Keep adapters and
   transport at their boundaries.
4. Add module-level tests for behavior at the same interface used by callers.
5. Run `.\echo.cmd verify`, `.\echo.cmd smoke`, and the required lower-level
   gates. Run authorized native acceptance for clipboard, activation, or paste
   changes.
6. Report each gate as PASS, FAIL, or NOT RUN. Do not call a candidate accepted;
   independent verification owns acceptance.

Changed paths are classified by `tools/echo`. Use
`.\echo.cmd verify --changed-from <base> --explain` to inspect the owner plan.

Storage verification is a single canonical gate. `.\echo.cmd verify storage`
runs the storage package tests together with the system-TEMP and repo-local
residue scanner. Full verification runs the non-storage workspace tests with
`--exclude echo-storage`, then invokes that same canonical storage gate before
the UI tests and build. A changed-owner plan containing `storage` also invokes
the canonical gate exactly once and does not schedule an independent
`cargo test -p echo-storage`; mixed storage/clipboard/tests plans retain the
same de-duplication.
