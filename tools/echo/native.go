package main

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

func (a *app) runNativeGate(scope string) error {
	if scope == "ui" {
		return a.runCoverFlowGate()
	}
	if runtime.GOOS != "windows" {
		return fmt.Errorf("native acceptance requires Windows")
	}
	if os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("set ECHO_WINDOWS_ACCEPTANCE=1 for separately authorized native acceptance")
	}
	if err := a.build(true); err != nil {
		return err
	}
	root := os.Getenv("ECHO_EVIDENCE_DIR")
	if root == "" {
		root = filepath.Join(a.root, ".local", "echo", "native-evidence")
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return err
	}
	run := filepath.Join(root, scope+"-"+time.Now().UTC().Format("20060102T150405.000000000"))
	fmt.Fprintf(a.out, "Native evidence: %s\n", run)
	tools := filepath.Join(root, "test-tools")
	if err := os.MkdirAll(tools, 0o700); err != nil {
		return err
	}
	overrides := map[string]string{}
	if os.Getenv("ECHO_NATIVE_FIXTURE_DIR") == "" && os.Getenv("ECHO_NATIVE_FIXTURE_EXE") == "" {
		if err := a.run("cargo", "build", "--release", "--locked", "-p", "echo-storage", "--example", "native_fixture"); err != nil {
			return err
		}
		overrides["ECHO_NATIVE_FIXTURE_EXE"] = filepath.Join(a.root, "target", "release", "examples", "native_fixture.exe")
	}
	if scope == "clipboard" || scope == "quick-insert" {
		if os.Getenv("ECHO_ACCEPTANCE_FIXTURE_EXE") == "" {
			fixture, err := a.buildNativeFixture(tools)
			if err != nil {
				return err
			}
			overrides["ECHO_ACCEPTANCE_FIXTURE_EXE"] = fixture
		}
	}
	return a.runWithEnv(overrides, "pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", filepath.Join(a.root, "tests", "native", "Invoke-NativeGate.ps1"),
		"-Root", a.root, "-Executable", a.desktopExecutable(true), "-Scope", scope, "-EvidenceRoot", run)
}

func forbiddenBrowserPackage(name string) bool {
	n := strings.ToLower(name)
	return n == "tauri" || strings.HasPrefix(n, "tauri-") || n == "wry" || n == "tao" || strings.HasPrefix(n, "webview2")
}
func (a *app) checkNoBrowserRuntime() error {
	for _, path := range []string{"apps/ui", "node_modules", "apps/desktop/gen", "apps/desktop/tauri.conf.json", "apps/desktop/capabilities", "apps/desktop/src/commands", "apps/desktop/src/transport", "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", "tests/ui", "tests/e2e"} {
		if _, err := os.Stat(filepath.Join(a.root, filepath.FromSlash(path))); err == nil {
			return fmt.Errorf("retired browser-stack path remains: %s", path)
		} else if !os.IsNotExist(err) {
			return err
		}
	}
	lock, err := os.ReadFile(filepath.Join(a.root, "Cargo.lock"))
	if err != nil {
		return err
	}
	for _, line := range strings.Split(string(lock), "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "name = ") {
			name := strings.Trim(strings.TrimPrefix(line, "name = "), "\"")
			if forbiddenBrowserPackage(name) {
				return fmt.Errorf("retired runtime remains in lockfile: %s", name)
			}
		}
	}
	for _, relative := range []string{"apps/desktop/src", "apps/desktop/ui", "crates/echo-presentation/src"} {
		err := filepath.WalkDir(filepath.Join(a.root, filepath.FromSlash(relative)), func(path string, entry os.DirEntry, walkErr error) error {
			if walkErr != nil {
				return walkErr
			}
			if entry.IsDir() {
				return nil
			}
			if filepath.Ext(path) != ".rs" && filepath.Ext(path) != ".slint" {
				return nil
			}
			content, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			for _, pattern := range []string{"tauri::", "@tauri-apps", "WebviewWindow", "wry::", "tao::", "register_uri_scheme_protocol"} {
				if strings.Contains(string(content), pattern) {
					return fmt.Errorf("retired UI bridge %q remains in %s", pattern, path)
				}
			}
			return nil
		})
		if err != nil {
			return err
		}
	}
	return nil
}
