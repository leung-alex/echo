package main

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"time"
)

// UI acceptance is a separately built, optimized test executable. Distribution builds
// never enable native-test. The bridge restricts all input/captures to owned synthetic data.
func (a *app) runSoftwareDeckGate() error {
	if runtime.GOOS != "windows" || os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("Software deck acceptance requires authorized Windows execution")
	}
	if err := a.run("cargo", "build", "-p", "echo-desktop", "--profile", "perf", "--no-default-features", "--features", "native-test", "--locked"); err != nil {
		return err
	}
	if err := a.run("cargo", "build", "-p", "echo-storage", "--release", "--example", "native_fixture", "--locked"); err != nil {
		return err
	}
	base := os.Getenv("ECHO_EVIDENCE_DIR")
	if base == "" {
		base = filepath.Join(a.root, ".local", "echo", "native-evidence")
	}
	run := filepath.Join(base, "software-deck-"+time.Now().UTC().Format("20060102T150405.000000000"))
	if err := os.MkdirAll(run, 0700); err != nil {
		return err
	}
	fixture := filepath.Join(a.root, "target", "release", "examples", "native_fixture.exe")
	template := filepath.Join(run, "fixtures")
	if err := a.run(fixture, template, "--software-deck"); err != nil {
		return err
	}
	exe := filepath.Join(a.root, "target", "perf", "echo-desktop.exe")
	for _, dataset := range []string{"T", "M"} {
		if err := a.run("pwsh", "-NoProfile", "-File", filepath.Join(a.root, "tests", "native", "Invoke-SoftwareDeckGate.ps1"),
			"-Root", a.root, "-Executable", exe, "-Template", filepath.Join(template, dataset),
			"-EvidenceRoot", filepath.Join(run, dataset)); err != nil {
			return err
		}
	}
	fmt.Fprintf(a.out, "Software card native evidence: %s\n", run)
	return nil
}
