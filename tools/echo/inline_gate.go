package main

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"time"
)

// Clipboard and Quick Insert share the current, owned-input acceptance runner.
// The diagnostic binary is isolated from distribution builds and user data.
func (a *app) runInlineGate(scope string) error {
	if scope != "clipboard" && scope != "quick-insert" {
		return fmt.Errorf("unsupported inline acceptance scope %q", scope)
	}
	if runtime.GOOS != "windows" || os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("native acceptance is separately authorized; set ECHO_WINDOWS_ACCEPTANCE=1 on Windows")
	}
	if err := a.run("cargo", "build", "-p", "echo-desktop", "--profile", "perf", "--features", "native-test", "--locked"); err != nil {
		return err
	}
	if err := a.run("cargo", "build", "-p", "echo-storage", "--release", "--example", "native_fixture", "--locked"); err != nil {
		return err
	}
	base := os.Getenv("ECHO_EVIDENCE_DIR")
	if base == "" {
		base = filepath.Join(a.root, ".local", "echo", "native-evidence")
	}
	if err := os.MkdirAll(base, 0700); err != nil {
		return err
	}
	evidence := filepath.Join(base, scope+"-"+time.Now().UTC().Format("20060102T150405.000000000"))
	fmt.Fprintf(a.out, "Native %s evidence: %s\n", scope, evidence)
	return a.run("pwsh", "-NoProfile", "-File", filepath.Join(a.root, "tests", "native", "Invoke-InlineGate.ps1"),
		"-Root", a.root, "-Executable", filepath.Join(a.root, "target", "perf", "echo-desktop.exe"),
		"-FixtureGenerator", filepath.Join(a.root, "target", "release", "examples", "native_fixture.exe"),
		"-Scope", scope, "-EvidenceRoot", evidence)
}
