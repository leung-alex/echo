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
		"crates/echo-presentation/src/lib.rs",
		"crates\\echo-storage\\src\\lib.rs",
		"tests/native/Invoke-UiAcceptance.ps1",
	})
	want := []string{"presentation", "quick-insert", "storage", "tests"}
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

func TestHistoricalPlannerAliasesUseConservativeFallback(t *testing.T) {
	plan := planForPaths([]string{"backend/crates/echo-storage/src/lib.rs", "frontend/app/main.ts"})
	if !plan.Full {
		t.Fatal("historical aliases did not select full verification")
	}
	if len(plan.Reasons) != 2 {
		t.Fatalf("unexpected fallback reasons: %#v", plan.Reasons)
	}
}

func TestArchitectureDependencyPolicyRejectsForbiddenEdges(t *testing.T) {
	metadata := []byte(`{"packages":[{"name":"echo-engine","dependencies":[{"name":"echo-storage"}]}]}`)
	if err := validateCargoArchitecture(metadata); err == nil {
		t.Fatal("forbidden engine-to-storage edge was accepted")
	}
}

func TestArchitectureDependencyPolicyAcceptsAdapterDirection(t *testing.T) {
	metadata := []byte(`{"packages":[{"name":"echo-storage","dependencies":[{"name":"echo-engine"}]},{"name":"echo-windows","dependencies":[{"name":"echo-engine"}]},{"name":"echo-engine","dependencies":[]}]}`)
	if err := validateCargoArchitecture(metadata); err != nil {
		t.Fatalf("valid adapter graph was rejected: %v", err)
	}
}

func TestCapturePathRejectsPerCaptureReconcile(t *testing.T) {
	source := "pub fn record_capture(&mut self) { self.reconcile_blob_store(); }\n"
	if err := rejectReconcileInCaptureFunctions(source); err == nil {
		t.Fatal("per-capture reconcile was accepted")
	}
}

func TestCapturePathAllowsExplicitRepair(t *testing.T) {
	source := "pub fn record_capture(&mut self) {}\npub fn reconcile_blob_store(&mut self) {}\n"
	if err := rejectReconcileInCaptureFunctions(source); err != nil {
		t.Fatalf("explicit repair was rejected: %v", err)
	}
}

func TestRootDependencyManifestsUseFullVerification(t *testing.T) {
	plan := planForPaths([]string{"Cargo.toml", "Cargo.lock"})
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

func TestStorageLeakScannerRejectsEchoResidue(t *testing.T) {
	root := t.TempDir()
	entryRoot := filepath.Join(root, "echo-storage-run")
	if err := os.MkdirAll(entryRoot, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(entryRoot, "echo.sqlite3-wal"), []byte("residue"), 0o644); err != nil {
		t.Fatal(err)
	}
	after := map[string]storageTempEntry{
		entryRoot: {path: entryRoot, isDir: true},
	}
	err := scanStorageLeak(map[string]storageTempEntry{}, after, nil)
	if err == nil || !strings.Contains(err.Error(), "echo.sqlite3-wal") {
		t.Fatalf("Echo residue was not rejected: %v", err)
	}
}

func TestStorageLeakScannerIgnoresGenericCodexTemp(t *testing.T) {
	root := t.TempDir()
	entryRoot := filepath.Join(root, ".tmpCodex123")
	if err := os.MkdirAll(entryRoot, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(entryRoot, "trace.txt"), []byte("generic"), 0o644); err != nil {
		t.Fatal(err)
	}
	after := map[string]storageTempEntry{
		entryRoot: {path: entryRoot, isDir: true},
	}
	if err := scanStorageLeak(map[string]storageTempEntry{}, after, nil); err != nil {
		t.Fatalf("generic Codex TEMP residue was rejected: %v", err)
	}
}

func TestStorageLeakScannerAcceptsCleanRepositoryRoot(t *testing.T) {
	if err := scanStorageLeak(
		map[string]storageTempEntry{},
		map[string]storageTempEntry{},
		nil,
	); err != nil {
		t.Fatalf("clean storage run was rejected: %v", err)
	}
}

type recordedCommand struct {
	name string
	args []string
}

func newRecordingApp(commands *[]recordedCommand, storageGates *int) *app {
	return &app{
		root:   filepath.Clean(`D:\Project\echo`),
		out:    &bytes.Buffer{},
		errOut: &bytes.Buffer{},
		runOverride: func(name string, args ...string) error {
			*commands = append(*commands, recordedCommand{name: name, args: append([]string(nil), args...)})
			return nil
		},
		storageGateOverride: func() error {
			(*storageGates)++
			return nil
		},
	}
}

func hasRecordedCommand(commands []recordedCommand, name string, args ...string) bool {
	for _, command := range commands {
		if command.name == name && reflect.DeepEqual(command.args, args) {
			return true
		}
	}
	return false
}

func hasGate(gates []string, want string) bool {
	for _, gate := range gates {
		if gate == want {
			return true
		}
	}
	return false
}

func TestStorageFocusedScopeRunsCanonicalGateOnce(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	if err := a.verifyScopeGates("storage"); err != nil {
		t.Fatalf("storage scope gates failed: %v", err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count = %d, want 1", storageGates)
	}
	if hasRecordedCommand(commands, "cargo", "test", "-p", "echo-storage", "--locked") {
		t.Fatal("focused storage scope ran an independent storage package test")
	}
	plan := OwnerPlan{Owners: []string{"storage"}}
	if !hasGate(ownerGateNames(plan, ValidationDeveloper), "echo.cmd verify storage") {
		t.Fatalf("storage planner omitted canonical gate: %v", ownerGateNames(plan, ValidationDeveloper))
	}
}

func TestStorageOwnerRunsCanonicalGateWithoutPackageDuplicate(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	plan := OwnerPlan{Owners: []string{"storage"}}
	if err := a.runOwnerPlanGates(plan, ValidationDeveloper); err != nil {
		t.Fatalf("storage owner gates failed: %v", err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count = %d, want 1", storageGates)
	}
	if hasRecordedCommand(commands, "cargo", "test", "-p", "echo-storage") {
		t.Fatal("storage owner ran an independent storage package test")
	}
	gates := ownerGateNames(plan, ValidationDeveloper)
	if !hasGate(gates, "echo.cmd verify storage") || hasGate(gates, "cargo test -p echo-storage") {
		t.Fatalf("storage planner gates = %v, want canonical storage gate only", gates)
	}
}

func TestMixedStorageClipboardRunsCanonicalGateOnce(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	plan := OwnerPlan{Owners: []string{"clipboard", "storage"}}
	if err := a.runOwnerPlanGates(plan, ValidationDeveloper); err != nil {
		t.Fatalf("mixed clipboard/storage gates failed: %v", err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count = %d, want 1", storageGates)
	}
	if !hasRecordedCommand(commands, "cargo", "test", "-p", "echo-engine", "--locked") || hasRecordedCommand(commands, "cargo", "test", "-p", "echo-storage") {
		t.Fatalf("mixed clipboard/storage package calls = %#v", commands)
	}
	gates := ownerGateNames(plan, ValidationDeveloper)
	if !hasGate(gates, "echo.cmd verify storage") || hasGate(gates, "cargo test -p echo-storage") {
		t.Fatalf("mixed clipboard/storage planner gates = %v", gates)
	}
}

func TestMixedStorageTestsRunsCanonicalGateOnce(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	plan := OwnerPlan{Owners: []string{"storage", "tests"}}
	if err := a.runOwnerPlanGates(plan, ValidationDeveloper); err != nil {
		t.Fatalf("mixed tests/storage gates failed: %v", err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count = %d, want 1", storageGates)
	}
	if hasRecordedCommand(commands, "cargo", "test", "-p", "echo-storage") {
		t.Fatal("mixed tests/storage ran an independent storage package test")
	}
	for _, packageName := range []string{"echo-engine", "echo-windows", "echo-activation"} {
		if !hasRecordedCommand(commands, "cargo", "test", "-p", packageName, "--locked") {
			t.Fatalf("mixed tests/storage omitted %s test: %#v", packageName, commands)
		}
	}
	if !hasRecordedCommand(commands, "cargo", "test", "-p", "echo-presentation", "--locked") || !hasRecordedCommand(commands, "cargo", "test", "-p", "echo-desktop", "--locked") {
		t.Fatalf("mixed tests/storage omitted UI or desktop gate: %#v", commands)
	}
	gates := ownerGateNames(plan, ValidationDeveloper)
	if hasGate(gates, "cargo test --workspace --locked") || hasGate(gates, "cargo test -p echo-storage") {
		t.Fatalf("mixed tests/storage planner retained duplicate workspace/storage gate: %v", gates)
	}
	for _, gate := range []string{
		"cargo test -p echo-engine --locked",
		"cargo test -p echo-windows --locked",
		"cargo test -p echo-activation --locked",
		"cargo test -p echo-presentation --locked",
		"cargo test -p echo-desktop --locked",
		"echo.cmd verify storage",
	} {
		if !hasGate(gates, gate) {
			t.Fatalf("mixed tests/storage planner omitted actual gate %q: %v", gate, gates)
		}
	}
}

func TestTestsOwnerPlannerMatchesCanonicalNativeExecution(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	plan := OwnerPlan{Owners: []string{"tests"}}
	if err := a.runOwnerPlanGates(plan, ValidationDeveloper); err != nil {
		t.Fatal(err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count=%d want1", storageGates)
	}
	gates := ownerGateNames(plan, ValidationDeveloper)
	for _, name := range []string{"echo-engine", "echo-windows", "echo-activation", "echo-presentation", "echo-desktop"} {
		if !hasRecordedCommand(commands, "cargo", "test", "-p", name, "--locked") {
			t.Errorf("native package not tested: %s", name)
		}
		if !hasGate(gates, "cargo test -p "+name+" --locked") {
			t.Errorf("planner omitted: %s", name)
		}
	}
	if !hasGate(gates, canonicalStorageGateName) || hasRecordedCommand(commands, "cargo", "test", "-p", "echo-storage", "--locked") {
		t.Fatal("storage gate not canonical/exclusive")
	}
	for _, cmd := range commands {
		if cmd.name == "pnpm" || cmd.name == "node" {
			t.Fatal("browser tooling remains")
		}
	}
}

func TestFullVerificationRunsExcludedWorkspaceAndCanonicalGateOnce(t *testing.T) {
	var commands []recordedCommand
	storageGates := 0
	a := newRecordingApp(&commands, &storageGates)
	if err := a.verifyProfileGates(ValidationDeveloper); err != nil {
		t.Fatalf("full verification gates failed: %v", err)
	}
	if storageGates != 1 {
		t.Fatalf("storage gate count = %d, want 1", storageGates)
	}
	if !hasRecordedCommand(commands, "cargo", "test", "--workspace", "--exclude", "echo-storage", "--locked") {
		t.Fatalf("full verification did not exclude echo-storage: %#v", commands)
	}
	if hasRecordedCommand(commands, "cargo", "test", "--workspace", "--locked") {
		t.Fatal("full verification also ran the unexcluded workspace test")
	}
	plan := OwnerPlan{Full: true}
	gates := ownerGateNames(plan, ValidationDeveloper)
	if !hasGate(gates, "cargo test --workspace --exclude echo-storage --locked") || !hasGate(gates, "echo.cmd verify storage") {
		t.Fatalf("full planner gates = %v", gates)
	}
}

func TestStorageLeakScannerRejectsNonEmptyRepositoryRoot(t *testing.T) {
	root := t.TempDir()
	repoRoot := filepath.Join(root, ".local", "test-tmp", "echo-storage")
	if err := os.MkdirAll(repoRoot, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(repoRoot, "leftover.txt"), []byte("residue"), 0o644); err != nil {
		t.Fatal(err)
	}
	repoEntries, err := snapshotStorageRepoEntries(repoRoot)
	if err != nil {
		t.Fatal(err)
	}
	if err := scanStorageLeak(map[string]storageTempEntry{}, map[string]storageTempEntry{}, repoEntries); err == nil || !strings.Contains(err.Error(), "leftover.txt") {
		t.Fatalf("repository-local residue was not rejected: %v", err)
	}
}

func TestCanonicalizeLineEndings(t *testing.T) {
	input := []byte("one\r\ntwo\rthree\nfour")
	want := []byte("one\ntwo\nthree\nfour")
	if got := canonicalizeLineEndings(input); !bytes.Equal(got, want) {
		t.Fatalf("canonicalizeLineEndings = %q, want %q", got, want)
	}
}
