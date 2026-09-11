package main

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestNativeAuditRejectsRetiredPackages(t *testing.T) {
	for _, name := range []string{"tauri", "tauri-runtime-wry", "wry", "tao", "webview2-com", "webview2-com-sys"} {
		if !forbiddenBrowserPackage(name) {
			t.Errorf("allowed browser dependency %s", name)
		}
	}
	for _, name := range []string{"slint", "winit", "windows-sys", "echo-engine", "raw-window-handle"} {
		if forbiddenBrowserPackage(name) {
			t.Errorf("rejected native dependency %s", name)
		}
	}
}
func TestNativeAuditFailsClosedOnLegacySourceAndLock(t *testing.T) {
	root := t.TempDir()
	for _, p := range []string{"apps/desktop/src", "apps/desktop/ui", "crates/echo-presentation/src"} {
		if err := os.MkdirAll(filepath.Join(root, p), 0755); err != nil {
			t.Fatal(err)
		}
	}
	lock := filepath.Join(root, "Cargo.lock")
	if err := os.WriteFile(lock, []byte("version = 4\n"), 0644); err != nil {
		t.Fatal(err)
	}
	a := &app{root: root}
	if err := a.checkNoBrowserRuntime(); err != nil {
		t.Fatal(err)
	}
	source := filepath.Join(root, "apps/desktop/src/accidental.rs")
	if err := os.WriteFile(source, []byte("use tauri::Manager;"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := a.checkNoBrowserRuntime(); err == nil {
		t.Fatal("old source accepted")
	}
	_ = os.Remove(source)
	if err := os.WriteFile(lock, []byte("[[package]]\nname = \"wry\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	if err := a.checkNoBrowserRuntime(); err == nil {
		t.Fatal("old runtime accepted")
	}
}
func TestNativePlansNeverRequireBrowserTools(t *testing.T) {
	for _, paths := range [][]string{{"apps/desktop/ui/app-window.slint"}, {"crates/echo-presentation/src/lib.rs"}, {"tests/native/EchoUi.cs"}, {"Cargo.lock"}, {"vendor/i-slint-backend-winit/accesskit.rs"}} {
		for _, gate := range ownerGateNames(planForPaths(paths), ValidationCI) {
			for _, old := range []string{"pnpm", "node ", "apps/ui", "tauri"} {
				if strings.Contains(gate, old) {
					t.Fatalf("retired command in plan: %s", gate)
				}
			}
		}
	}
}

func TestNativeGateRoutesToCurrentRunners(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Windows command routing")
	}
	t.Setenv("ECHO_WINDOWS_ACCEPTANCE", "1")
	for _, scope := range []string{"smoke", "ui", "clipboard", "quick-insert"} {
		t.Run(scope, func(t *testing.T) {
			sentinel := errors.New("stop before executing build")
			var command string
			a := &app{root: t.TempDir(), out: io.Discard, runOverride: func(name string, args ...string) error {
				command = name + " " + strings.Join(args, " ")
				return sentinel
			}}
			if err := a.runNativeGate(scope); !errors.Is(err, sentinel) {
				t.Fatalf("route failed: %v", err)
			}
			expected := "--features native-test"
			if scope == "smoke" {
				expected = "--release"
			}
			if !strings.Contains(command, expected) {
				t.Fatalf("%s routed to %s", scope, command)
			}
		})
	}
}

func TestSmokeDoesNotRequireMutatingAcceptance(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Windows smoke")
	}
	t.Setenv("ECHO_WINDOWS_ACCEPTANCE", "")
	sentinel := errors.New("build reached")
	a := &app{root: t.TempDir(), out: io.Discard, runOverride: func(string, ...string) error { return sentinel }}
	if err := a.smoke(); !errors.Is(err, sentinel) {
		t.Fatalf("smoke rejected before build: %v", err)
	}
	if err := a.runNativeGate("obsolete"); err == nil || errors.Is(err, sentinel) {
		t.Fatalf("unknown scope reached build: %v", err)
	}
}
