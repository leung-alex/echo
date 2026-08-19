# C05 — Remove Clipboard/Quick Insert ownership from Culsans

**Blocked by:** C02, E05

**What to build**

Delete embedded clipboard listener, library, Quick Insert, business storage and settings ownership.

**Acceptance criteria**

- [ ] Culsans no longer listens to clipboard for Echo business behavior
- [ ] Culsans no longer opens/writes Echo business tables
- [ ] Quick Insert implementation is removed
- [ ] Commands launch Echo
- [ ] Culsans-local tests remain green

**Status:** ready-for-agent
