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
	case "cargo.toml", "cargo.lock":
		return true
	default:
		return false
	}
}

func ownersForPath(path string) (bool, []string) {
	path = strings.ToLower(normalizeRepoPath(path))
	switch {
	case path == "echo.cmd", path == "cargo.toml", path == "cargo.lock":
		return true, []string{"tooling"}
	case strings.HasPrefix(path, "tools/"), strings.HasPrefix(path, ".github/workflows/"):
		return true, []string{"tooling"}
	case strings.HasPrefix(path, "vendor/"):
		return false, nil
	case strings.HasPrefix(path, "crates/echo-presentation/"):
		return true, []string{"presentation", "quick-insert"}
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
	case strings.HasPrefix(path, "tests/native/"):
		return true, []string{"tests", "quick-insert"}
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

// A single plan drives display and execution so --explain never advertises
// different tests and every storage run includes the canonical leak gate.
func ownerPackages(plan OwnerPlan) ([]string, bool) {
	packages := map[string]bool{}
	storage := contains(plan.Owners, "storage")
	add := func(names ...string) {
		for _, name := range names {
			packages[name] = true
		}
	}
	for _, owner := range plan.Owners {
		switch owner {
		case "clipboard", "engine":
			add("echo-engine", "echo-windows")
			storage = true
		case "windows":
			add("echo-windows")
		case "library":
			add("echo-engine")
		case "quick-insert":
			add("echo-engine", "echo-windows", "echo-activation", "echo-presentation", "echo-desktop")
		case "presentation":
			add("echo-presentation", "echo-desktop")
		case "desktop", "activation":
			add("echo-desktop", "echo-activation")
		case "tests":
			add("echo-engine", "echo-windows", "echo-activation", "echo-presentation", "echo-desktop")
			storage = true
		}
	}
	names := make([]string, 0, len(packages))
	for name := range packages {
		names = append(names, name)
	}
	sort.Strings(names)
	return names, storage
}
func ownerGateNames(plan OwnerPlan, profile ValidationProfile) []string {
	gates := []string{"echo.cmd self-check", "echo.cmd format --check", "go test ./...", "go vet ./..."}
	if plan.Full {
		gates = append(gates, "cargo test --workspace --exclude echo-storage --locked", canonicalStorageGateName)
	} else {
		packages, storage := ownerPackages(plan)
		for _, name := range packages {
			gates = append(gates, "cargo test -p "+name+" --locked")
		}
		if storage {
			gates = append(gates, canonicalStorageGateName)
		}
	}
	if profile == ValidationCI {
		gates = append(gates, "echo.cmd build --release")
	}
	return gates
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
