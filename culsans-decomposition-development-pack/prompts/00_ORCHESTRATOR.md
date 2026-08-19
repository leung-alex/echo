# Codex Prompt — Decomposition Orchestrator

You are coordinating a four-repository architecture cutover.

Read the entire development pack before acting.

Repositories:

```text
D:\Projects\culsans
D:\Projects\aster
D:\Projects\iris
D:\Projects\echo
```

Hard requirement: these are fully independent sibling Git repositories. There is no parent workspace and there must be no cross-repository source/path dependency.

Your job is to:

1. Record the current Culsans baseline SHA and baseline build/test commands.
2. Confirm each repository has its own Git history.
3. Create/maintain a cutover status document containing:
   - Culsans SHA
   - Aster SHA
   - Iris SHA
   - Echo SHA
   - local gate status
   - integration smoke status
   - migration status
4. Enforce the external activation contract in `docs/03_EXTERNAL_APP_PROTOCOL_V1.md`.
5. Keep four implementation streams parallel:
   - Aster
   - Iris
   - Echo
   - Culsans runtime hollowing
6. Prevent sequential full-regression cycles.
7. Schedule one integration smoke gate and one final full acceptance.
8. Reject any proposal that introduces:
   - sibling path dependency,
   - shared DB,
   - shared mutable data dir,
   - synchronized product versions,
   - giant common SDK/runtime.

Do not implement product features unless required to resolve integration ownership.
