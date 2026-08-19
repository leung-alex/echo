# C06 — Final runtime, storage and contract contraction

**Blocked by:** C03, C04, C05

**What to build**

Remove obsolete fields/APIs/contracts/migrations/dependencies so the remaining Culsans code expresses only shell ownership.

**Acceptance criteria**

- [ ] Runtime contains no Aster/Iris/Echo business services
- [ ] Culsans storage contains only shell-owned data
- [ ] External-product DTOs are removed from Culsans contracts
- [ ] No dead compatibility adapters remain
- [ ] Workspace manifests no longer list migrated product crates

**Status:** ready-for-agent
