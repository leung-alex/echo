# Codex Operating Rules

## 1. Do not redesign while extracting

Preserve externally visible behavior unless the current behavior blocks independence.

Log redesign ideas separately.

## 2. Read before move

Each implementation agent must first inventory:

- source modules
- tests
- settings
- persistence
- Tauri commands
- frontend routes
- platform helpers
- bundled resources

Then produce a short migration map in its repo before edits.

## 3. Move ownership, not only files

A feature is migrated only when:

- destination owns implementation,
- destination owns lifecycle,
- destination owns data,
- destination owns settings,
- destination owns tests,
- Culsans no longer owns it.

## 4. No sibling source dependency

Before every milestone, search manifests for:

```text
../culsans
../aster
../iris
../echo
```

Any build dependency found is a blocker.

## 5. No shared mutable DB

Never point a new app at the legacy Culsans DB as its permanent store.

Reading legacy DB for migration is allowed.

## 6. Local green, not global green

Implementation agents run their own repo tests.

Do not repeatedly run full Culsans acceptance unless changing shell/input behavior.

## 7. Commit discipline

Prefer small, intention-revealing commits:

```text
scaffold standalone Aster desktop
move Everything lifecycle into Aster
move File Search UI into Aster
add Aster activation v1
remove File Search runtime ownership from Culsans
```

Avoid “big refactor” commits containing unrelated formatting.

## 8. Integration defects return to owner

If integration reviewer finds an Aster defect, assign it to Aster agent.

Reviewer should not become the owner of every cross-product fix.

## 9. Preserve baseline evidence

Do not delete old tests before equivalent destination behavior is proven.

Move/adapt tests first where practical, then remove obsolete Culsans tests.

## 10. Stop conditions

An agent must stop and report instead of inventing a coupling workaround if it believes it needs:

- a shared cross-repo source library,
- a shared database,
- bidirectional runtime RPC,
- a synchronized product version,
- a permanent compatibility adapter in Culsans.

Those require architecture review.
