package main

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestParseNameStatusZ(t *testing.T) {
	output := []byte("M\x00crates/echo-engine/src/ingest.rs\x00R100\x00old.ts\x00tests/ui/new.ts\x00A\x00tests/e2e/clipboard.spec.ts\x00")
	changes, err := parseNameStatusZ(output)
	if err != nil {
		t.Fatalf("parseNameStatusZ returned error: %v", err)
	}
	want := []ChangedPath{
		{Path: "crates/echo-engine/src/ingest.rs", Status: ChangeModified},
		{Path: "old.ts", Status: ChangeDeleted},
		{Path: "tests/e2e/clipboard.spec.ts", Status: ChangeAdded},
		{Path: "tests/ui/new.ts", Status: ChangeAdded},
	}
	if !reflect.DeepEqual(changes, want) {
		t.Fatalf("changes = %#v, want %#v", changes, want)
	}
}

func TestPlanForPathsMapsOwnersDeterministically(t *testing.T) {
	plan := planForPaths([]string{
		"apps/ui/src/main.ts",
		"crates\\echo-storage\\src\\lib.rs",
		"tests/e2e/quick-insert.spec.ts",
	})
	want := []string{"frontend", "library", "migration", "quick-insert", "storage", "tests"}
	if !reflect.DeepEqual(plan.Owners, want) {
		t.Fatalf("owners = %#v, want %#v", plan.Owners, want)
	}
	if plan.Full || len(plan.Reasons) != 0 {
		t.Fatalf("unexpected conservative fallback: %#v", plan)
	}
}

func TestUnknownPathUsesConservativeFullPlan(t *testing.T) {
	plan := planForPaths([]string{"scripts/unknown.bin"})
	if !plan.Full {
		t.Fatal("unknown path did not select full verification")
	}
	if len(plan.Reasons) != 1 || !strings.Contains(plan.Reasons[0], "scripts/unknown.bin") {
		t.Fatalf("unexpected fallback reasons: %#v", plan.Reasons)
	}
}

func TestRootDependencyManifestsUseFullVerification(t *testing.T) {
	plan := planForPaths([]string{"Cargo.toml", "pnpm-lock.yaml"})
	if !plan.Full || !reflect.DeepEqual(plan.Owners, []string{"tooling"}) {
		t.Fatalf("root manifests were not full verification: %#v", plan)
	}
}

func TestFindRepositoryRoot(t *testing.T) {
	root := t.TempDir()
	for _, relative := range []string{"crates", "apps", "crates/echo-engine"} {
		if err := os.MkdirAll(filepath.Join(root, relative), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(root, "Cargo.toml"), []byte("[workspace]\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	nested := filepath.Join(root, "crates", "echo-engine")
	found, err := findRepositoryRoot(nested)
	if err != nil {
		t.Fatalf("findRepositoryRoot returned error: %v", err)
	}
	if found != root {
		t.Fatalf("root = %q, want %q", found, root)
	}
}

func TestCommandStringQuotesArguments(t *testing.T) {
	got := commandString("pnpm", "--dir", "apps/ui", "run", "test suite")
	want := `pnpm --dir apps/ui run "test suite"`
	if got != want {
		t.Fatalf("commandString = %q, want %q", got, want)
	}
}

func TestOwnedCleanPathsStayInsideRepository(t *testing.T) {
	root := filepath.Clean(`D:\Project\echo`)
	for _, path := range ownedCleanPaths(root) {
		relative, err := filepath.Rel(root, path)
		if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(os.PathSeparator)) {
			t.Fatalf("clean path escaped root: %q", path)
		}
	}
}

func TestPrintPlanIncludesConservativeReason(t *testing.T) {
	a := &app{out: &bytes.Buffer{}, errOut: &bytes.Buffer{}}
	plan := planForPaths([]string{"unknown/file.dat"})
	a.printPlan(plan, ValidationDeveloper)
	output := a.out.(*bytes.Buffer).String()
	if !strings.Contains(output, "Conservative fallback reasons") || !strings.Contains(output, "unknown/file.dat") {
		t.Fatalf("plan output omitted fallback reason: %s", output)
	}
}

func TestAcceptanceFailsClosedWithoutAuthorization(t *testing.T) {
	t.Setenv("ECHO_WINDOWS_ACCEPTANCE", "")
	a := &app{out: &bytes.Buffer{}, errOut: &bytes.Buffer{}}
	if err := a.acceptanceCommand([]string{"clipboard"}); err == nil || !strings.Contains(err.Error(), "separately authorized") {
		t.Fatalf("acceptance did not fail closed: %v", err)
	}
}

func TestGeneratedBindingsDriftCheckFailsClosed(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, filepath.FromSlash(generatedTransportPath))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("stale"), 0o644); err != nil {
		t.Fatal(err)
	}
	a := &app{root: root, out: &bytes.Buffer{}, errOut: &bytes.Buffer{}}
	if err := a.checkGeneratedBindings(); err == nil {
		t.Fatal("stale generated bindings were accepted")
	}
	if err := os.WriteFile(path, []byte(generatedTransportBindings()), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := a.checkGeneratedBindings(); err != nil {
		t.Fatalf("fresh generated bindings were rejected: %v", err)
	}
	windows := bytes.ReplaceAll([]byte(generatedTransportBindings()), []byte{'\n'}, []byte{'\r', '\n'})
	if err := os.WriteFile(path, windows, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := a.checkGeneratedBindings(); err != nil {
		t.Fatalf("Windows line endings were rejected: %v", err)
	}
}

func TestCanonicalizeLineEndings(t *testing.T) {
	input := []byte("one\r\ntwo\rthree\nfour")
	want := []byte("one\ntwo\nthree\nfour")
	if got := canonicalizeLineEndings(input); !bytes.Equal(got, want) {
		t.Fatalf("canonicalizeLineEndings = %q, want %q", got, want)
	}
}
