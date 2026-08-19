# Target Architecture

## 1. Problem Statement

Culsans has accumulated multiple complete user workflows inside one product/runtime:

- command shell and global input
- file search
- clipboard/history/snippets/insertion
- capture/pin/drawing
- settings
- browser integration
- window/focus behavior

The problem is not only source volume. The current composition root and runtime own too many long-lived services, while the desktop and frontend packages directly depend on unrelated product capabilities.

This increases:

- change blast radius
- merge conflicts
- agent context requirements
- full-regression frequency
- package/install size
- runtime failure coupling
- ownership ambiguity

## 2. Solution

Turn Culsans into a focused **Command Shell / Control Plane** and move three complete workflows into standalone products:

- **Aster — Find**
- **Iris — See**
- **Echo — Recall**

They are not plugins inside a Culsans monorepo. They are independent applications that Culsans may launch.

## 3. Physical architecture

```text
D:\Projects\
├── culsans\   (.git)
├── aster\     (.git)
├── iris\      (.git)
└── echo\      (.git)
```

No parent workspace exists.

## 4. Product ownership

### Culsans owns

- Command Panel
- app discovery / app registry
- semantic command mapping
- application launcher
- global input / gesture
- Input Agent bridge and protocol
- browser integration
- shell-level settings
- global shortcuts
- shell presentation / overlays that are genuinely shell-level
- current Windows/window/focus context needed by the shell

### Aster owns

- File Search UX
- File Search domain model
- filters and search scope
- Everything process / SDK integration
- Everything bundled resources/config
- search diagnostics
- indexing/prewarm policy
- search-specific settings
- search-specific tests
- any future tag/content-search features that belong to Aster

### Iris owns

- screenshot capture
- precision capture
- capture editor
- pin
- annotation
- drawing
- drawing library
- Fabric
- Excalidraw
- image export
- visual-tool persistence
- visual-tool settings

### Echo owns

- Windows clipboard listener
- clipboard ingestion
- representations and blob persistence
- history
- favorites
- snippets
- Quick Insert UI
- copy/insert actions
- paste target validation
- background maintenance / blob GC
- retention/privacy settings

## 5. Independence invariant

A product is considered independent only when it satisfies:

```text
can clone/build/test/run/release
without the source directories of the other products
```

A build that requires `../culsans`, `../aster`, `../iris`, or `../echo` violates the architecture.

## 6. Dependency direction

Allowed:

```text
Culsans --process activation--> Aster
Culsans --process activation--> Iris
Culsans --process activation--> Echo
```

Not allowed:

```text
Aster -> culsans-runtime
Iris  -> culsans-platform
Echo  -> culsans-storage
Culsans -> aster crate
Culsans -> iris frontend package
Culsans -> echo database
```

## 7. Deliberate code duplication

Small duplication is acceptable at the start.

Prefer:

- 30 lines of duplicated Windows helper code

over:

- a cross-repository shared crate that forces coordinated releases.

Only create a shared library later when:

1. at least two products need the same stable behavior,
2. the abstraction has stopped changing,
3. the dependency can be versioned independently,
4. the ownership and release policy are explicit.

Do **not** create `culsans-common`, `culsans-shared`, `culsans-ui`, or `culsans-sdk` during this extraction unless a blocker proves it is unavoidable.

## 8. Runtime topology

Recommended steady state:

```text
culsans.exe
  └─ lightweight shell, usually resident

culsans-input-agent.exe
  └─ resident input owner

aster.exe
  ├─ UI on demand
  └─ search backend lifecycle owned by Aster

iris.exe
  └─ on demand by default

echo-agent.exe / echo background host
  └─ resident clipboard owner when history is enabled

echo.exe
  └─ Quick Insert UI on demand
```

Exact Echo process packaging can evolve, but the architectural invariant is that the clipboard listener is not owned by Culsans.

## 9. Architecture anti-goals

Do not produce:

- a distributed `culsans-runtime`
- a giant bidirectional IPC bus
- shared business DTO packages across all repos
- a single shared settings database
- one installer that must rebuild all four products
- a version lock such as `Culsans 0.5 requires Aster 0.5 requires Iris 0.5`
- a temporary architecture intended to live across multiple releases

This migration should cut directly to the target ownership model.
