# X01 — Rehearse data migration on legacy copy

**Blocked by:** E02, I04

**What to build**

Run Echo/Iris migration against a disposable copy of representative legacy Culsans data.

**Acceptance criteria**

- [ ] Backup is created
- [ ] Echo row/blob invariants pass
- [ ] Iris drawing invariants pass
- [ ] Rerun is idempotent
- [ ] No original data is modified during rehearsal

**Status:** ready-for-agent
