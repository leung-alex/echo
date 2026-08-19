# Codex Prompt — Culsans Shell Contraction Agent

Workspace: `D:\Projects\culsans`

Read the full pack and `docs/04_CULSANS_PLAN.md`.

Mission:

Turn Culsans into the shell/control plane while Aster, Iris and Echo are being extracted in parallel.

You own:

- external app registry/discovery,
- activation encoding/launch,
- semantic command routing,
- removal of migrated business ownership,
- runtime hollowing,
- storage cleanup,
- frontend dependency cleanup,
- installer cleanup.

Hard rules:

- do not recreate a giant AppManager/runtime;
- Culsans does not inspect app internals;
- do not keep compatibility services after destination ownership is proven;
- do not keep dual-write storage;
- do not bundle Everything after Aster cutover;
- remove Iris-only frontend dependencies;
- remove Echo clipboard lifecycle;
- preserve Input Agent and shell hot path.

Work with external agents by consuming their declared activation actions and readiness checkpoints.

Do not wait for all three apps before beginning shell infrastructure. Build the external seam immediately, then contract ownership as each destination reaches its local gate.

Do not perform a full regression after each contraction. Run Culsans-local tests and integration smoke only.
