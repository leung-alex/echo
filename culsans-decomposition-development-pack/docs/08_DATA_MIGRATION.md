# Data Migration Plan

## Principle

Perform **one user-visible architecture migration**, not a sequence of temporary schemas.

Avoid:

```text
Culsans-only
→ Culsans + Aster
→ Culsans + Aster + Iris
→ Culsans + Aster + Iris + Echo
```

as separately supported data states.

Target directly:

```text
Culsans data → Culsans-owned data only
Iris data    → Iris-owned data
Echo data    → Echo-owned data
Aster data   → Aster-owned data
```

## Known current storage facts

Current Culsans storage includes:

- command/application-related migrations
- clipboard migrations
- saved insert items
- drawing persistence

The current clipboard store opens:

```text
<culsans data dir>\culsans.sqlite3
<culsans data dir>\blobs\
```

Drawing documents/library are also persisted through Culsans storage.

## Target data locations

```text
%LOCALAPPDATA%\Culsans\
%LOCALAPPDATA%\Aster\
%LOCALAPPDATA%\Iris\
%LOCALAPPDATA%\Echo\
```

Recommended:

```text
Culsans\culsans.sqlite3
Iris\iris.sqlite3
Echo\echo.sqlite3
Echo\blobs\
```

Aster chooses only what it genuinely needs.

## Ownership map

### Remain in Culsans

Only shell-owned tables/configuration such as:

- command/application launch state
- shell settings
- input/browser/shell state if persisted there

### Move to Echo

- clipboard settings
- clipboard entries
- clipboard representations
- pinned/favorite state
- saved insert items
- snippets
- blob files

### Move to Iris

- drawing documents
- drawing library
- drawing thumbnails
- future visual history that belongs to Iris

### Move to Aster

- search settings/state owned by Aster
- future search catalog/tag data

## Migration implementation

Create a one-time migration utility or startup migrators owned by the destination apps.

Preferred sequence:

1. Detect legacy Culsans database.
2. Acquire a migration lock.
3. Ensure Culsans/Echo/Iris are not concurrently writing legacy business data.
4. Back up the entire legacy DB and blob directory.
5. Create destination DBs.
6. Copy data transactionally.
7. Verify row counts and referential invariants.
8. Verify all referenced clipboard blobs exist or record missing blobs explicitly.
9. Mark migration completion with a destination-owned migration marker.
10. Do not delete the legacy backup automatically.
11. Start destination products from new data.
12. Culsans stops opening migrated business tables.

## Idempotency

Re-running migration must:

- not duplicate rows,
- not corrupt blobs,
- detect already migrated schema,
- verify rather than blindly recopy when safe,
- fail with actionable diagnostics if source changed unexpectedly.

## Compatibility window

The first post-cutover build may retain **read-only migration detection** for the old Culsans data.

It should not retain active dual-write.

Forbidden:

```text
write clipboard to Culsans DB
AND
write clipboard to Echo DB
```

Dual-write creates ownership ambiguity and is not part of the target architecture.

## Migration acceptance

- legacy History preserved
- Favorites preserved
- Snippets preserved
- rich representations preserved
- images/blobs preserved
- Echo settings preserved
- drawing documents preserved
- drawing library preserved
- Culsans shell data unchanged
- destination apps work after source backup is moved aside
