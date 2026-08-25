package main

import (
	"bytes"
	"fmt"
	"path/filepath"
	"sort"
	"strings"
)

type ValidationProfile string

const (
	ValidationDeveloper ValidationProfile = "developer"
	ValidationCI        ValidationProfile = "ci"
)

const (
	canonicalStorageGateName = "echo.cmd verify storage"
	storagePackageGateName   = "cargo test -p echo-storage"
)

type ChangeStatus string

const (
	ChangeAdded    ChangeStatus = "added"
	ChangeDeleted  ChangeStatus = "deleted"
	ChangeModified ChangeStatus = "modified"
)

type ChangedPath struct {
	Path   string
	Status ChangeStatus
}

type OwnerPlan struct {
	Full    bool
	Owners  []string
	Changes []ChangedPath
	Reasons []string
}

func planForPaths(paths []string) OwnerPlan {
	plan := OwnerPlan{}
	owners := map[string]bool{}
	for _, raw := range paths {
		path := normalizeRepoPath(raw)
		if path == "" {
			continue
		}
		if isFullBoundaryPath(path) {
			plan.Full = true
			owners["tooling"] = true
			continue
		}
		matched, pathOwners := ownersForPath(path)
		if !matched {
			plan.Full = true
			plan.Reasons = append(plan.Reasons, "unclassified path: "+path)
			continue
		}
		for _, owner := range pathOwners {
			owners[owner] = true
		}
	}
	for owner := range owners {
		plan.Owners = append(plan.Owners, owner)
	}
	sort.Strings(plan.Owners)
	plan.Reasons = uniqueSorted(plan.Reasons)
	return plan
}

func isFullBoundaryPath(path string) bool {
	switch strings.ToLower(normalizeRepoPath(path)) {
	case "cargo.toml", "cargo.lock", "pnpm-lock.yaml", "pnpm-workspace.yaml":
		return true
	default:
		return false
	}
}

func ownersForPath(path string) (bool, []string) {
	path = strings.ToLower(normalizeRepoPath(path))
	switch {
	case path == "echo.cmd", path == "package.json", path == "pnpm-lock.yaml", path == "pnpm-workspace.yaml", path == "cargo.toml", path == "cargo.lock":
		return true, []string{"tooling"}
	case strings.HasPrefix(path, "tools/echo/"):
		return true, []string{"tooling"}
	case strings.HasPrefix(path, "crates/echo-engine/"):
		return true, []string{"engine", "desktop"}
	case strings.HasPrefix(path, "crates/echo-storage/"):
		return true, []string{"storage"}
	case strings.HasPrefix(path, "crates/echo-windows/"):
		return true, []string{"windows", "desktop"}
	case strings.HasPrefix(path, "crates/echo-activation/"):
		return true, []string{"desktop", "activation"}
	case strings.HasPrefix(path, "apps/desktop/"):
		return true, []string{"desktop", "activation", "quick-insert"}
	case strings.HasPrefix(path, "apps/ui/"):
		return true, []string{"frontend", "quick-insert"}
	case strings.HasPrefix(path, "tests/e2e/clipboard"):
		return true, []string{"tests", "clipboard"}
	case strings.HasPrefix(path, "tests/e2e/quick-insert"):
		return true, []string{"tests", "quick-insert"}
	case strings.HasPrefix(path, "tests/ui/"):
		return true, []string{"tests", "frontend"}
	case strings.HasPrefix(path, "tests/"):
		return true, []string{"tests"}
	case path == "docs/test_ownership_map.md", path == "docs/p06_p07_handoff.md":
		return true, []string{"tests", "tooling"}
	case strings.HasPrefix(path, "docs/") || strings.HasPrefix(path, "echo-complete-development-pack/"):
		return true, nil
	case path == ".gitignore":
		return true, []string{"tooling"}
	default:
		return false, nil
	}
}

func ownerGateNames(plan OwnerPlan, profile ValidationProfile) []string {
	if plan.Full {
		gates := []string{
			"echo.cmd self-check",
			"echo.cmd format --check",
			"go test ./...",
			"go vet ./...",
			"cargo test --workspace --exclude echo-storage --locked",
			canonicalStorageGateName,
			"pnpm --dir apps/ui test",
			"pnpm --dir apps/ui build",
		}
		if profile == ValidationCI {
			gates = append(gates, "echo.cmd build --release", "echo.cmd package --dir")
		}
		return gates
	}

	gateSet := map[string]bool{
		"echo.cmd self-check":     true,
		"echo.cmd format --check": true,
		"go test ./...":           true,
		"go vet ./...":            true,
	}
	needStorageGate := contains(plan.Owners, "storage")
	for _, owner := range plan.Owners {
		switch owner {
		case "clipboard", "engine":
			gateSet["cargo test -p echo-engine"] = true
			if !needStorageGate {
				gateSet[storagePackageGateName] = true
			}
		case "storage":
		case "library":
			gateSet["cargo test -p echo-engine"] = true
		case "quick-insert":
			gateSet["cargo test -p echo-engine"] = true
			gateSet["pnpm --dir apps/ui test"] = true
		case "frontend":
			gateSet["pnpm --dir apps/ui test"] = true
			gateSet["pnpm --dir apps/ui build"] = true
		case "desktop", "activation":
			gateSet["cargo check -p echo-desktop"] = true
		case "windows":
			gateSet["cargo test -p echo-windows"] = true
		case "tests":
			gateSet["cargo test -p echo-engine"] = true
			gateSet["cargo test -p echo-windows"] = true
			gateSet["cargo test -p echo-activation"] = true
			if !needStorageGate {
				gateSet[storagePackageGateName] = true
			}
			gateSet["pnpm --dir apps/ui test"] = true
			gateSet["cargo check -p echo-desktop"] = true
		case "tooling":
			gateSet["echo.cmd self-check"] = true
		}
	}
	if needStorageGate {
		gateSet[canonicalStorageGateName] = true
	}
	if profile == ValidationCI {
		gateSet["echo.cmd build --release"] = true
	}
	result := make([]string, 0, len(gateSet))
	for gate := range gateSet {
		result = append(result, gate)
	}
	sort.Strings(result)
	return result
}

func normalizeRepoPath(path string) string {
	if strings.TrimSpace(path) == "" {
		return ""
	}
	path = strings.ReplaceAll(path, "\\", "/")
	path = strings.TrimPrefix(path, "./")
	path = filepath.ToSlash(filepath.Clean(path))
	if path == "." {
		return ""
	}
	return path
}

func parseNameStatusZ(output []byte) ([]ChangedPath, error) {
	tokens := bytes.Split(output, []byte{0})
	changes := make([]ChangedPath, 0)
	for index := 0; index < len(tokens); {
		if len(tokens[index]) == 0 {
			index++
			continue
		}
		statusToken := string(tokens[index])
		index++
		status := statusToken
		firstPath := ""
		if tab := strings.IndexByte(statusToken, '\t'); tab >= 0 {
			status = statusToken[:tab]
			firstPath = statusToken[tab+1:]
		}
		if firstPath == "" {
			if index >= len(tokens) || len(tokens[index]) == 0 {
				return nil, fmt.Errorf("git change %q is missing its path", status)
			}
			firstPath = string(tokens[index])
			index++
		}
		if status == "" {
			return nil, fmt.Errorf("git change has an empty status")
		}
		switch status[0] {
		case 'R':
			if index >= len(tokens) || len(tokens[index]) == 0 {
				return nil, fmt.Errorf("rename %q is missing its destination", firstPath)
			}
			changes = append(changes,
				ChangedPath{Path: firstPath, Status: ChangeDeleted},
				ChangedPath{Path: string(tokens[index]), Status: ChangeAdded},
			)
			index++
		case 'C':
			if index >= len(tokens) || len(tokens[index]) == 0 {
				return nil, fmt.Errorf("copy %q is missing its destination", firstPath)
			}
			changes = append(changes, ChangedPath{Path: string(tokens[index]), Status: ChangeAdded})
			index++
		case 'A':
			changes = append(changes, ChangedPath{Path: firstPath, Status: ChangeAdded})
		case 'D':
			changes = append(changes, ChangedPath{Path: firstPath, Status: ChangeDeleted})
		default:
			changes = append(changes, ChangedPath{Path: firstPath, Status: ChangeModified})
		}
	}
	return mergeChangedPaths(changes), nil
}

func mergeChangedPaths(changes []ChangedPath) []ChangedPath {
	merged := map[string]ChangedPath{}
	for _, change := range changes {
		change.Path = normalizeRepoPath(change.Path)
		if change.Path == "" {
			continue
		}
		key := strings.ToLower(change.Path)
		if existing, ok := merged[key]; ok {
			if existing.Status == ChangeDeleted || change.Status == ChangeDeleted {
				change.Status = ChangeDeleted
			} else if existing.Status == ChangeAdded || change.Status == ChangeAdded {
				change.Status = ChangeAdded
			}
		}
		merged[key] = change
	}
	result := make([]ChangedPath, 0, len(merged))
	for _, change := range merged {
		result = append(result, change)
	}
	sort.Slice(result, func(i, j int) bool {
		return strings.ToLower(result[i].Path) < strings.ToLower(result[j].Path)
	})
	return result
}

func uniqueSorted(values []string) []string {
	seen := map[string]bool{}
	for _, value := range values {
		if value != "" {
			seen[value] = true
		}
	}
	result := make([]string, 0, len(seen))
	for value := range seen {
		result = append(result, value)
	}
	sort.Strings(result)
	return result
}
