# E02 — Establish Echo storage, blobs and migration

**Blocked by:** E01

**What to build**

Move clipboard persistence, representations, saved items, snippets, settings and blob ownership to Echo.

**Acceptance criteria**

- [ ] Echo uses Echo-owned DB and blob directory
- [ ] Legacy data migration is backup-first and idempotent
- [ ] History/Favorites/Snippets survive migration
- [ ] Blob references are verified
- [ ] No permanent Culsans DB writes

**Status:** ready-for-agent
