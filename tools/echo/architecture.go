package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type cargoMetadataPackage struct {
	Name         string                    `json:"name"`
	Dependencies []cargoMetadataDependency `json:"dependencies"`
}

type cargoMetadataDependency struct {
	Name string `json:"name"`
}

type cargoMetadataDocument struct {
	Packages []cargoMetadataPackage `json:"packages"`
}

type forbiddenSourcePattern struct {
	name  string
	value string
}

func (a *app) checkArchitectureContracts(metadata []byte) error {
	if err := validateCargoArchitecture(metadata); err != nil {
		return err
	}
	if err := a.checkNativeUIBoundary(); err != nil {
		return err
	}
	if err := a.checkActiveArchitectureResidue(); err != nil {
		return err
	}
	if err := a.checkStorageArchitecture(); err != nil {
		return err
	}
	if err := a.checkCanonicalCommandSurface(); err != nil {
		return err
	}
	if err := a.checkLocalityContract(); err != nil {
		return err
	}
	return a.checkHistoricalRouting()
}

func validateCargoArchitecture(metadata []byte) error {
	var document cargoMetadataDocument
	if err := json.Unmarshal(metadata, &document); err != nil {
		return fmt.Errorf("parse cargo metadata for architecture audit: %w", err)
	}
	allowed := map[string]map[string]bool{
		"echo-engine":       {},
		"echo-storage":      {"echo-engine": true},
		"echo-windows":      {"echo-engine": true},
		"echo-activation":   {},
		"echo-presentation": {"echo-engine": true},
		"echo-desktop-ui":   {},
		"echo-icon-assets":  {},
		"echo-desktop": {
			"echo-icon-assets":  true,
			"echo-desktop-ui":   true,
			"echo-activation":   true,
			"echo-presentation": true,
			"echo-engine":       true,
			"echo-storage":      true,
			"echo-windows":      true,
		},
	}
	for _, packageInfo := range document.Packages {
		allowedDependencies, checked := allowed[packageInfo.Name]
		if !checked {
			continue
		}
		for _, dependency := range packageInfo.Dependencies {
			if strings.HasPrefix(dependency.Name, "echo-") && !allowedDependencies[dependency.Name] {
				return fmt.Errorf("forbidden Echo dependency edge: %s -> %s", packageInfo.Name, dependency.Name)
			}
		}
	}
	return nil
}

// Presentation is pure Rust; compiled Slint belongs only to the desktop host.
func (a *app) checkNativeUIBoundary() error {
	for _, layer := range []string{"echo-engine", "echo-presentation"} {
		files, err := textFilesUnder(filepath.Join(a.root, "crates", layer, "src"))
		if err != nil {
			return err
		}
		for _, file := range files {
			data, err := os.ReadFile(file)
			if err != nil {
				return err
			}
			for _, forbidden := range []string{"use slint", "use tauri", "use echo_storage", "use echo_windows", "windows_sys::", "rusqlite::"} {
				if bytes.Contains(data, []byte(forbidden)) {
					return fmt.Errorf("pure %s imports adapter/UI dependency %q in %s", layer, forbidden, file)
				}
			}
		}
	}
	return nil
}

func (a *app) checkActiveArchitectureResidue() error {
	patterns := []forbiddenSourcePattern{
		{name: "obsolete activation flag", value: "--culsans-activate"},
		{name: "obsolete image command", value: "quick_insert_get_image"},
		{name: "historical crate path", value: "backend/crates/"},
		{name: "historical frontend path", value: "frontend/app/"},
		{name: "removed reusable-content feature", value: "snippet"},
	}
	for _, root := range []string{"crates", "apps"} {
		files, err := textFilesUnder(filepath.Join(a.root, root))
		if err != nil {
			return err
		}
		for _, file := range files {
			content, err := os.ReadFile(file)
			if err != nil {
				return fmt.Errorf("read active source %s: %w", relativeToRoot(a.root, file), err)
			}
			lower := strings.ToLower(string(content))
			for _, pattern := range patterns {
				if strings.Contains(lower, strings.ToLower(pattern.value)) {
					return fmt.Errorf("active %s remains in %s", pattern.name, relativeToRoot(a.root, file))
				}
			}
			relative := filepath.ToSlash(relativeToRoot(a.root, file))
			previewTransport := strings.HasPrefix(relative, "apps/desktop/ui/") ||
				relative == "crates/echo-engine/src/preview.rs"
			if previewTransport {
				for _, pattern := range []string{"base64", "data:image", "todataurl"} {
					if strings.Contains(lower, pattern) {
						return fmt.Errorf("preview transport contains forbidden %s in %s", pattern, relative)
					}
				}
			}
			if strings.HasPrefix(relative, "apps/desktop/") {
				for _, pattern := range []string{"setinterval", "pollhistory", "history_poll", "favorites_poll", "background_poll", "900ms"} {
					if strings.Contains(lower, pattern) {
						return fmt.Errorf("history polling residue %q remains in %s", pattern, relative)
					}
				}
			}
		}
	}
	storage, err := os.ReadFile(filepath.Join(a.root, "crates", "echo-storage", "src", "lib.rs"))
	if err != nil {
		return fmt.Errorf("read storage capture path: %w", err)
	}
	if err := rejectReconcileInCaptureFunctions(string(storage)); err != nil {
		return err
	}
	return nil
}

func textFilesUnder(root string) ([]string, error) {
	var files []string
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			switch strings.ToLower(entry.Name()) {
			case ".git", "target", "node_modules", "dist", ".local":
				return filepath.SkipDir
			}
			return nil
		}
		switch strings.ToLower(filepath.Ext(path)) {
		case ".rs", ".slint", ".go", ".toml", ".json", ".cmd":
			files = append(files, path)
		}
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("scan active source %s: %w", root, err)
	}
	return files, nil
}

func rejectReconcileInCaptureFunctions(source string) error {
	for _, marker := range []string{"pub fn record_capture", "pub fn record_captured", "fn record_captured"} {
		start := 0
		for {
			index := strings.Index(source[start:], marker)
			if index < 0 {
				break
			}
			index += start
			end := len(source)
			for _, boundary := range []string{"\n    pub fn ", "\n    fn ", "\npub fn ", "\nfn ", "\n#[cfg(test)]"} {
				if candidate := strings.Index(source[index+len(marker):], boundary); candidate >= 0 {
					candidate += index + len(marker)
					if candidate < end {
						end = candidate
					}
				}
			}
			if strings.Contains(source[index:end], "reconcile_blob_store") {
				return fmt.Errorf("per-capture reconcile remains in %q", marker)
			}
			start = index + len(marker)
		}
	}
	return nil
}

func (a *app) checkStorageArchitecture() error {
	storagePath := filepath.Join(a.root, "crates", "echo-storage", "src", "lib.rs")
	storage, err := os.ReadFile(storagePath)
	if err != nil {
		return fmt.Errorf("read storage architecture: %w", err)
	}
	content := string(storage)
	for _, required := range []string{
		"struct WriterRuntime",
		"struct ReaderRuntime",
		"clipboard_fts MATCH",
		"saved_items_fts MATCH",
	} {
		if !strings.Contains(content, required) {
			return fmt.Errorf("storage architecture is missing %q", required)
		}
	}
	engine, err := os.ReadFile(filepath.Join(a.root, "crates", "echo-engine", "src", "lib.rs"))
	if err != nil {
		return fmt.Errorf("read engine public surface: %w", err)
	}
	if bytes.Contains(engine, []byte("pub mod ")) ||
		bytes.Contains(engine, []byte("pub struct MemorySink")) ||
		bytes.Contains(engine, []byte("echo_storage")) {
		return fmt.Errorf("engine exposes a module or storage implementation edge")
	}
	for _, forbidden := range []string{"pub fn with_store", "pub fn from_store"} {
		if strings.Contains(content, forbidden) {
			return fmt.Errorf("storage leaks an internal runtime surface: %s", forbidden)
		}
	}
	return nil
}

func (a *app) checkCanonicalCommandSurface() error {
	read := func(relative string) ([]byte, error) {
		content, err := os.ReadFile(filepath.Join(a.root, filepath.FromSlash(relative)))
		if err != nil {
			return nil, fmt.Errorf("read canonical command source %s: %w", relative, err)
		}
		return content, nil
	}
	readme, err := read("README.md")
	if err != nil {
		return err
	}
	for _, required := range []string{
		"echo.cmd self-check",
		"echo.cmd acceptance ui",
		"echo.cmd format --check",
		"echo.cmd perf",
		"--echo-activate",
	} {
		if !bytes.Contains(readme, []byte(required)) {
			return fmt.Errorf("README is missing canonical contract %q", required)
		}
	}
	for _, relative := range []string{"echo.cmd", "README.md", "crates/echo-activation/src/lib.rs"} {
		content, err := read(relative)
		if err != nil {
			return err
		}
		if bytes.Contains(content, []byte("--culsans-activate")) {
			return fmt.Errorf("obsolete activation flag remains in %s", relative)
		}
	}
	return nil
}

func (a *app) checkLocalityContract() error {
	path := filepath.Join(a.root, "docs", "architecture", "locality.md")
	content, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("read locality contract: %w", err)
	}
	text := string(content)
	sections := []struct {
		name    string
		primary string
		adapter string
	}{
		{"thumbnail size", "crates/echo-engine/src/preview.rs", "crates/echo-storage/src/lib.rs"},
		{"Quick Insert paste", "crates/echo-engine/src/quick_insert.rs", "crates/echo-windows/src/lib.rs"},
		{"History search ranking", "crates/echo-storage/src/lib.rs", "crates/echo-engine/src/history.rs"},
	}
	for _, section := range sections {
		start := strings.Index(strings.ToLower(text), strings.ToLower("## "+section.name))
		if start < 0 {
			return fmt.Errorf("locality contract is missing %s", section.name)
		}
		end := strings.Index(text[start+3:], "\n## ")
		if end < 0 {
			end = len(text) - start - 3
		}
		block := text[start : start+3+end]
		if !strings.Contains(block, "Primary module:") || !strings.Contains(block, section.primary) {
			return fmt.Errorf("locality contract has no primary module for %s", section.name)
		}
		if strings.Count(block, "Adapter:") != 1 || !strings.Contains(block, section.adapter) {
			return fmt.Errorf("locality contract must name exactly one adapter for %s", section.name)
		}
	}
	return nil
}

func (a *app) checkHistoricalRouting() error {
	for _, relative := range []string{
		"docs/P06_P07_HANDOFF.md",
		"docs/archive/P08_VISUAL_HANDOFF.md",
		"docs/VISUAL_PARITY_DEVIATIONS.md",
	} {
		content, err := os.ReadFile(filepath.Join(a.root, filepath.FromSlash(relative)))
		if err != nil {
			return fmt.Errorf("read historical routing document %s: %w", relative, err)
		}
		trimmed := strings.TrimSpace(string(content))
		if !strings.HasPrefix(trimmed, "> Historical record only.") {
			return fmt.Errorf("historical document is not clearly labeled: %s", relative)
		}
	}
	return nil
}
