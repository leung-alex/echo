# Echo P06/P07 Handoff

Current HEAD:

## P06

Legacy tests mapped:

Destination tests:

Tests that stay in Culsans:

Tests replaced/obsolete with reason:

## P07

Public entry:

Commands:

Owner graph:

Go tests:

## Required gate results

```text
<root-command> self-check
<root-command> format --check
<root-command> verify
<root-command> smoke
<root-command> acceptance ...
<root-command> package
go test ./...
go vet ./...
```

Results:

## Culsans cleanup now safe

Tests:

`cu` product gates:

Shared helpers requiring partial cleanup:

## Blockers

## Final commit SHA
