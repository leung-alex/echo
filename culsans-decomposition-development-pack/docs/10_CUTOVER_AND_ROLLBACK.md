# Cutover and Rollback Plan

## Cutover prerequisites

All must be true:

- Aster local gate green
- Iris local gate green
- Echo local gate green
- Culsans shell local gate green
- activation contract v1 implemented consistently
- integration smoke gate green
- migration tested on a copy of real legacy data
- legacy baseline commit/tag recorded
- install/uninstall paths known

## Cutover sequence

1. Stop active Culsans background writers.
2. Back up Culsans data.
3. Run Echo migration.
4. Run Iris migration.
5. Initialize Aster state/resources.
6. Build/install Aster.
7. Build/install Iris.
8. Build/install Echo.
9. Build/install hollowed Culsans.
10. Start Culsans.
11. Verify app discovery.
12. Smoke Aster launch.
13. Smoke Iris launch.
14. Smoke Echo launch.
15. Verify migrated data.
16. Run final full acceptance.

## Commit/release structure

Use explicit target commits in all four repos.

Record a manifest:

```text
culsans: <sha>
aster:   <sha>
iris:    <sha>
echo:    <sha>
```

This is not version locking; it records what was tested together for the cutover.

## Rollback triggers

Rollback if any of these occur:

- data migration corruption/loss
- global input regression that risks stuck keys/modifiers
- Culsans cannot reliably launch apps
- Echo paste targets become unsafe/unreliable
- capture becomes unusable
- Aster cannot search on supported environments
- uninstall/install topology damages another product

## Rollback procedure

1. Stop all four products.
2. Preserve failed post-cutover data for diagnosis.
3. Restore legacy Culsans data backup.
4. Restore the recorded legacy Culsans build.
5. Do not merge post-cutover Echo/Iris data back into legacy automatically.
6. Diagnose using preserved migration logs and failed destination data.

## Anti-pattern

Do not “rollback” by reintroducing random individual features into the hollowed runtime.

Rollback is a whole architecture-state rollback, not ad-hoc dual ownership.
