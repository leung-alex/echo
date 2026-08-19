# I04 — Establish Iris storage and settings ownership

**Blocked by:** I03

**What to build**

Create Iris-owned visual persistence/settings and support migration of legacy drawing data.

**Acceptance criteria**

- [ ] Iris writes `iris.sqlite3` or equivalent Iris-owned store
- [ ] Drawing documents/library persist across restart
- [ ] Legacy drawing import is idempotent
- [ ] Iris visual settings are independent

**Status:** ready-for-agent
