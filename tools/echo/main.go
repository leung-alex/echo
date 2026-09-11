package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"syscall"
	"time"
)

const requiredGoVersion = "go1.26.2"

const helpText = `Echo repository tool

Usage:
  echo.cmd help
      Show this public command reference.
  echo.cmd install
      Fetch the locked Rust dependencies (no Node or browser runtime).
  echo.cmd tokens [--check]
      Generate or validate native Slint/Rust and reference CSS design tokens.
  echo.cmd format [--check]
      Format or check Go and Rust source files.
  echo.cmd verify [--changed-from <sha>] [--profile <developer|ci>] [--explain]
      Run full verification, or verify only owners affected by Git changes.
  echo.cmd verify clipboard
      Run the focused Clipboard ownership gates.
  echo.cmd verify quick-insert
      Run the focused Quick Insert ownership gates.
  echo.cmd verify storage
      Run the focused storage tests and global TEMP leak gate.
  echo.cmd self-check
      Validate repository automation and independence invariants.
  echo.cmd build [--release]
      Build the native Slint desktop executable.
  echo.cmd dev
      Start Echo with the native Slint renderer.
  echo.cmd smoke
      Run authorized Release startup and graceful shutdown with synthetic data.
  echo.cmd perf
      Run the deterministic storage diagnostic and print JSON output.
  echo.cmd acceptance <clipboard|quick-insert|ui>
      Run separately authorized native Windows acceptance.
  echo.cmd package [--dir]
      Build portable ZIP and optional NSIS installer, or only the release executable.
  echo.cmd release-candidate
      Verify clean inputs and produce a local release candidate manifest.
  echo.cmd sync [--all | --branch <fixed-branch>]
      Show fixed worktree status and select interactively; scripts must use --all or --branch.
  echo.cmd clean
      Remove only Echo-owned generated bootstrap and run state.

Commands without an explicit selection never mutate Git non-interactively.
`

type app struct {
	root          string
	in            io.Reader
	out           io.Writer
	errOut        io.Writer
	env           []string
	syncMainPath  string
	syncWorktrees []fixedWorktree
	// Test-only seams. Production construction leaves both nil so command and
	// storage verification use the real runners below.
	runOverride         func(string, ...string) error
	storageGateOverride func() error
}

type exitError struct {
	code int
	err  error
}

func (e *exitError) Error() string { return e.err.Error() }
func (e *exitError) Unwrap() error { return e.err }

func main() {
	a, err := newApp()
	if err == nil {
		err = a.dispatch(os.Args[1:])
	}
	if err == nil {
		return
	}
	fmt.Fprintln(os.Stderr, "ERROR:", err)
	var exit *exitError
	if errors.As(err, &exit) {
		os.Exit(exit.code)
	}
	os.Exit(1)
}

func newApp() (*app, error) {
	root, err := findRepositoryRoot(mustGetwd())
	if err != nil {
		return nil, err
	}
	return &app{
		root:          root,
		in:            os.Stdin,
		out:           os.Stdout,
		errOut:        os.Stderr,
		env:           os.Environ(),
		syncMainPath:  mainWorktreePath,
		syncWorktrees: fixedWorktrees,
	}, nil
}

func (a *app) dispatch(args []string) error {
	if len(args) == 0 || args[0] == "help" || args[0] == "-h" || args[0] == "--help" {
		_, _ = io.WriteString(a.out, helpText)
		return nil
	}
	switch args[0] {
	case "install":
		return noArgs(args[1:], a.install)
	case "tokens":
		fs := newFlags("tokens", a.errOut)
		check := fs.Bool("check", false, "verify generated tokens without edits")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		return a.generateTokens(*check)
	case "format":
		fs := newFlags("format", a.errOut)
		check := fs.Bool("check", false, "check formatting without edits")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		return a.format(*check)
	case "verify":
		return a.verifyCommand(args[1:])
	case "self-check":
		return noArgs(args[1:], a.selfCheck)
	case "build":
		fs := newFlags("build", a.errOut)
		release := fs.Bool("release", false, "build release artifacts")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		return a.build(*release)
	case "dev":
		return noArgs(args[1:], a.dev)
	case "smoke":
		return noArgs(args[1:], a.smoke)
	case "perf":
		return noArgs(args[1:], a.perf)
	case "acceptance":
		return a.acceptanceCommand(args[1:])
	case "package":
		fs := newFlags("package", a.errOut)
		dir := fs.Bool("dir", false, "build only the unpackaged release executable")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		return a.packageCommand(*dir)
	case "release-candidate":
		return noArgs(args[1:], a.releaseCandidate)
	case "sync":
		return a.syncCommand(args[1:])
	case "clean":
		return noArgs(args[1:], a.clean)
	default:
		return fmt.Errorf("unknown command %q\n\n%s", args[0], helpText)
	}
}

func newFlags(name string, output io.Writer) *flag.FlagSet {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	fs.SetOutput(output)
	return fs
}

func parseFlags(fs *flag.FlagSet, args []string) error {
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return fmt.Errorf("unexpected arguments: %s", strings.Join(fs.Args(), " "))
	}
	return nil
}

func noArgs(args []string, fn func() error) error {
	if len(args) != 0 {
		return fmt.Errorf("unexpected arguments: %s", strings.Join(args, " "))
	}
	return fn()
}

func mustGetwd() string {
	wd, err := os.Getwd()
	if err != nil {
		return "."
	}
	return wd
}

func findRepositoryRoot(start string) (string, error) {
	path, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	if info, statErr := os.Stat(path); statErr == nil && !info.IsDir() {
		path = filepath.Dir(path)
	}
	for {
		if fileExists(filepath.Join(path, "Cargo.toml")) &&
			directoryExists(filepath.Join(path, "crates")) &&
			directoryExists(filepath.Join(path, "apps")) {
			return path, nil
		}
		parent := filepath.Dir(path)
		if parent == path {
			return "", fmt.Errorf("Echo repository root not found from %s", start)
		}
		path = parent
	}
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

func directoryExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func (a *app) verifyCommand(args []string) error {
	fs := newFlags("verify", a.errOut)
	changedFrom := fs.String("changed-from", "", "verify owners affected since a commit")
	profileValue := fs.String("profile", string(ValidationDeveloper), "developer or ci")
	explain := fs.Bool("explain", false, "print the plan without running it")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if *changedFrom != "" {
		if fs.NArg() != 0 {
			return fmt.Errorf("--changed-from cannot be combined with a verification scope")
		}
		profile, err := parseProfile(*profileValue)
		if err != nil {
			return err
		}
		return a.verifyChanged(*changedFrom, profile, *explain)
	}
	if *explain || *profileValue != string(ValidationDeveloper) {
		return fmt.Errorf("--profile and --explain require --changed-from")
	}
	if fs.NArg() == 0 {
		return a.verifyProfile(ValidationDeveloper)
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("verify accepts at most one scope")
	}
	switch fs.Arg(0) {
	case "clipboard", "quick-insert", "storage":
		return a.verifyScope(fs.Arg(0))
	default:
		return fmt.Errorf("unknown verify scope %q", fs.Arg(0))
	}
}

func parseProfile(value string) (ValidationProfile, error) {
	profile := ValidationProfile(value)
	if profile != ValidationDeveloper && profile != ValidationCI {
		return "", fmt.Errorf("unknown validation profile %q", value)
	}
	return profile, nil
}

func (a *app) verifyChanged(base string, profile ValidationProfile, explain bool) error {
	changes, err := a.collectChangedPaths(base)
	if err != nil {
		return err
	}
	paths := make([]string, 0, len(changes))
	for _, change := range changes {
		paths = append(paths, change.Path)
	}
	plan := planForPaths(paths)
	plan.Changes = changes
	if len(changes) == 0 {
		plan.Owners = []string{"tooling"}
	}
	a.printPlan(plan, profile)
	if explain {
		return nil
	}
	return a.runOwnerPlan(plan, profile)
}

func (a *app) printPlan(plan OwnerPlan, profile ValidationProfile) {
	fmt.Fprintf(a.out, "ECHO VERIFICATION PLAN\nProfile: %s\n", profile)
	if len(plan.Changes) == 0 {
		fmt.Fprintln(a.out, "Changed files\n  (none)")
	} else {
		fmt.Fprintln(a.out, "Changed files")
		for _, change := range plan.Changes {
			fmt.Fprintf(a.out, "  - %s (%s)\n", change.Path, change.Status)
		}
	}
	fmt.Fprintln(a.out, "Owners")
	if plan.Full {
		fmt.Fprintln(a.out, "  - full")
	}
	for _, owner := range plan.Owners {
		fmt.Fprintf(a.out, "  - %s\n", owner)
	}
	if len(plan.Reasons) != 0 {
		fmt.Fprintln(a.out, "Conservative fallback reasons")
		for _, reason := range plan.Reasons {
			fmt.Fprintf(a.out, "  - %s\n", reason)
		}
	}
	fmt.Fprintln(a.out, "Gates")
	for _, gate := range ownerGateNames(plan, profile) {
		fmt.Fprintf(a.out, "  - %s\n", gate)
	}
}

func (a *app) verifyScope(scope string) error {
	if err := a.runVerificationPrelude(); err != nil {
		return err
	}
	return a.verifyScopeGates(scope)
}

func (a *app) runVerificationPrelude() error {
	if err := a.selfCheck(); err != nil {
		return err
	}
	if err := a.format(true); err != nil {
		return err
	}
	return a.runGoChecks()
}

func (a *app) verifyScopeGates(scope string) error {
	if scope == "storage" {
		return a.runStorageGate()
	}
	packages := []string{"echo-engine", "echo-presentation", "echo-activation", "echo-desktop"}
	if scope == "clipboard" {
		packages = []string{"echo-engine", "echo-windows", "echo-desktop"}
	} else if scope != "quick-insert" {
		return fmt.Errorf("unknown verification scope %q", scope)
	}
	for _, p := range packages {
		if err := a.run("cargo", "test", "-p", p, "--locked"); err != nil {
			return err
		}
	}
	if scope == "clipboard" {
		return a.runStorageGate()
	}
	return nil
}

type storageTempEntry struct {
	path  string
	size  int64
	isDir bool
}

func snapshotStorageTempTopLevel(root string) (map[string]storageTempEntry, error) {
	entries := make(map[string]storageTempEntry)
	children, err := os.ReadDir(root)
	if os.IsNotExist(err) {
		return entries, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read storage TEMP root %s: %w", root, err)
	}
	for _, child := range children {
		path := filepath.Join(root, child.Name())
		info, err := child.Info()
		if err != nil {
			return nil, fmt.Errorf("stat storage TEMP entry %s: %w", path, err)
		}
		entry := storageTempEntry{path: path, isDir: info.IsDir()}
		if !info.IsDir() {
			entry.size = info.Size()
		}
		entries[path] = entry
	}
	return entries, nil
}

func newStorageTempEntries(
	before, after map[string]storageTempEntry,
) []storageTempEntry {
	entries := make([]storageTempEntry, 0)
	for path, entry := range after {
		if _, existed := before[path]; !existed {
			entries = append(entries, entry)
		}
	}
	sort.Slice(entries, func(left, right int) bool {
		return entries[left].path < entries[right].path
	})
	return entries
}

func isEchoStorageResidue(path string) bool {
	name := strings.ToLower(filepath.Base(path))
	if strings.HasPrefix(name, "echo-") {
		return true
	}
	switch name {
	case "echo.sqlite", "echo.sqlite3", "echo.sqlite-wal", "echo.sqlite3-wal", "echo.sqlite-shm", "echo.sqlite3-shm":
		return true
	default:
		return false
	}
}

func scanNewStorageTempEntry(entry storageTempEntry) ([]string, error) {
	residue := make([]string, 0)
	visit := func(path string) error {
		if isEchoStorageResidue(path) {
			residue = append(residue, path)
		}
		return nil
	}
	if err := visit(entry.path); err != nil {
		return nil, err
	}
	if !entry.isDir {
		return residue, nil
	}
	if err := filepath.WalkDir(entry.path, func(path string, _ os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if path == entry.path {
			return nil
		}
		return visit(path)
	}); err != nil {
		return nil, fmt.Errorf("scan new storage TEMP entry %s: %w", entry.path, err)
	}
	return residue, nil
}

func snapshotStorageRepoEntries(root string) ([]string, error) {
	if !directoryExists(root) {
		return nil, nil
	}
	entries := make([]string, 0)
	if err := filepath.WalkDir(root, func(path string, _ os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if path != root {
			relative, err := filepath.Rel(root, path)
			if err != nil {
				return err
			}
			entries = append(entries, filepath.ToSlash(relative))
		}
		return nil
	}); err != nil {
		return nil, fmt.Errorf("scan repository storage test root %s: %w", root, err)
	}
	sort.Strings(entries)
	return entries, nil
}

func scanStorageLeak(
	before, after map[string]storageTempEntry,
	repoEntries []string,
) error {
	newEntries := newStorageTempEntries(before, after)
	residue := make([]string, 0)
	for _, entry := range newEntries {
		found, err := scanNewStorageTempEntry(entry)
		if err != nil {
			return err
		}
		residue = append(residue, found...)
	}
	if len(residue) != 0 {
		return fmt.Errorf("Echo storage residue found in system TEMP: %s", strings.Join(residue, ", "))
	}
	if len(repoEntries) != 0 {
		return fmt.Errorf("repository-local storage test root was not cleaned: %s", strings.Join(repoEntries, ", "))
	}
	return nil
}

func (a *app) runStorageGate() error {
	started := time.Now()
	fmt.Fprintln(a.out, "Echo storage verification start")
	tempRoot := os.TempDir()
	repoRoot := filepath.Join(a.root, ".local", "test-tmp", "echo-storage")
	beforeTemp, err := snapshotStorageTempTopLevel(tempRoot)
	if err != nil {
		return err
	}
	if _, err := snapshotStorageRepoEntries(repoRoot); err != nil {
		return err
	}
	if a.storageGateOverride != nil {
		return a.storageGateOverride()
	}
	runErr := a.run("cargo", "test", "-p", "echo-storage", "--locked")
	afterTemp, snapshotErr := snapshotStorageTempTopLevel(tempRoot)
	if snapshotErr != nil {
		return snapshotErr
	}
	repoEntries, repoErr := snapshotStorageRepoEntries(repoRoot)
	if repoErr != nil {
		return repoErr
	}
	if runErr != nil {
		return runErr
	}
	if err := scanStorageLeak(beforeTemp, afterTemp, repoEntries); err != nil {
		return err
	}
	newEntries := newStorageTempEntries(beforeTemp, afterTemp)
	fmt.Fprintf(
		a.out,
		"Echo storage verification passed; TEMP new top-level=%d; repo-local residual=%d; wall=%s\n",
		len(newEntries),
		len(repoEntries),
		time.Since(started).Round(time.Millisecond),
	)
	return nil
}

func (a *app) verifyProfile(profile ValidationProfile) error {
	if err := a.runVerificationPrelude(); err != nil {
		return err
	}
	return a.verifyProfileGates(profile)
}

func (a *app) verifyProfileGates(profile ValidationProfile) error {
	if err := a.run("cargo", "test", "--workspace", "--exclude", "echo-storage", "--locked"); err != nil {
		return err
	}
	if err := a.runStorageGate(); err != nil {
		return err
	}
	if profile == ValidationCI {
		return a.build(true)
	}
	return nil
}

func (a *app) runOwnerPlan(plan OwnerPlan, profile ValidationProfile) error {
	if err := a.runVerificationPrelude(); err != nil {
		return err
	}
	return a.runOwnerPlanGates(plan, profile)
}

func (a *app) runOwnerPlanGates(plan OwnerPlan, profile ValidationProfile) error {
	if plan.Full {
		return a.verifyProfileGates(profile)
	}
	packages, storage := ownerPackages(plan)
	for _, name := range packages {
		if err := a.run("cargo", "test", "-p", name, "--locked"); err != nil {
			return err
		}
	}
	if storage {
		if err := a.runStorageGate(); err != nil {
			return err
		}
	}
	if profile == ValidationCI {
		return a.build(true)
	}
	return nil
}

func contains(values []string, value string) bool {
	for _, candidate := range values {
		if candidate == value {
			return true
		}
	}
	return false
}

func (a *app) collectChangedPaths(base string) ([]ChangedPath, error) {
	diff, err := a.gitOutput("diff", "--name-status", "-z", "--find-renames", base, "--")
	if err != nil {
		return nil, err
	}
	changes, err := parseNameStatusZ(diff)
	if err != nil {
		return nil, err
	}
	untracked, err := a.gitOutput("ls-files", "--others", "--exclude-standard", "-z")
	if err != nil {
		return nil, err
	}
	for _, token := range bytes.Split(untracked, []byte{0}) {
		if len(token) != 0 {
			changes = append(changes, ChangedPath{Path: string(token), Status: ChangeAdded})
		}
	}
	return mergeChangedPaths(changes), nil
}

func (a *app) selfCheck() error {
	if err := requireGoVersion(); err != nil {
		return err
	}
	for _, path := range []string{"echo.cmd", "Cargo.toml", "Cargo.lock", "apps/desktop/ui/app-window.slint", "apps/desktop/resources/echo.manifest", "apps/desktop/icons/icon.ico", "tools/echo/go.mod", "docs/TEST_OWNERSHIP_MAP.md"} {
		if !fileExists(filepath.Join(a.root, filepath.FromSlash(path))) {
			return fmt.Errorf("self-check expected file is missing: %s", path)
		}
	}
	if fileExists(filepath.Join(a.root, ".gitmodules")) {
		return fmt.Errorf("self-check rejects Git submodules")
	}
	if err := a.checkManifestIndependence(); err != nil {
		return err
	}
	metadata, err := a.runCapture("cargo", "metadata", "--no-deps", "--locked", "--format-version", "1")
	if err != nil {
		return err
	}
	if containsForbiddenDependency(metadata) {
		return fmt.Errorf("sibling dependency in Cargo metadata")
	}
	if err = a.checkArchitectureContracts([]byte(metadata)); err != nil {
		return err
	}
	if err = a.checkNativeFixtureSources(); err != nil {
		return err
	}
	if err := a.generateTokens(true); err != nil {
		return err
	}
	return a.checkNoBrowserRuntime()
}

func (a *app) checkNativeFixtureSources() error {
	for _, path := range []string{"tests/native/EchoUi.cs", "tests/native/EchoDriver.cs", "tests/native/Invoke-UiAcceptance.ps1", "tests/native/Invoke-NativeGate.ps1", "tools/echo/fixture/main_windows.go", "tools/echo/fixture/target_windows.go"} {
		if !fileExists(filepath.Join(a.root, filepath.FromSlash(path))) {
			return fmt.Errorf("native fixture source is missing: %s", path)
		}
	}
	return nil
}

type echoTokenOccurrence struct {
	token      string
	definition bool
}

func echoTokenOccurrences(content string) []echoTokenOccurrence {
	var tokens []echoTokenOccurrence
	for offset := 0; offset < len(content); {
		index := strings.Index(content[offset:], "--echo-")
		if index < 0 {
			break
		}
		start := offset + index
		end := start + len("--echo-")
		for end < len(content) {
			character := content[end]
			if (character >= 'a' && character <= 'z') || (character >= 'A' && character <= 'Z') || (character >= '0' && character <= '9') || character == '-' || character == '_' {
				end++
				continue
			}
			break
		}
		token := content[start:end]
		tokens = append(tokens, echoTokenOccurrence{
			token:      token,
			definition: strings.HasPrefix(strings.TrimSpace(content[end:]), ":"),
		})
		offset = end
	}
	return tokens
}

func requireGoVersion() error {
	output, err := exec.Command("go", "env", "GOVERSION").Output()
	if err != nil {
		return fmt.Errorf("read Go version: %w", err)
	}
	version := strings.TrimSpace(string(output))
	if version != requiredGoVersion {
		return fmt.Errorf("Echo requires Go %s; found %s", requiredGoVersion, version)
	}
	return nil
}

func (a *app) checkManifestIndependence() error {
	return filepath.WalkDir(a.root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			n := entry.Name()
			if n == ".git" || n == "target" || n == "node_modules" || n == ".local" || n == "dist" || filepath.ToSlash(relativeToRoot(a.root, path)) == "tests/verification" {
				return filepath.SkipDir
			}
			return nil
		}
		switch entry.Name() {
		case "Cargo.toml", "Cargo.lock", "echo.cmd", "go.mod":
			content, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			if containsForbiddenDependency(string(content)) {
				return fmt.Errorf("sibling dependency found in %s", relativeToRoot(a.root, path))
			}
		}
		return nil
	})
}

func containsForbiddenDependency(content string) bool {
	lower := strings.ToLower(strings.ReplaceAll(content, "\\", "/"))
	for _, forbidden := range []string{
		"path = \"../",
		"path='../",
		"file:../",
		"file:..//",
	} {
		if strings.Contains(lower, forbidden) {
			return true
		}
	}
	return false
}

func (a *app) checkGeneratedIgnore(relative string) error {
	cmd := exec.Command("git", "check-ignore", "--quiet", relative)
	cmd.Dir = a.root
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("generated file is not ignored: %s", relative)
	}
	return nil
}

func (a *app) format(check bool) error {
	files, err := a.goFiles()
	if err != nil {
		return err
	}
	if check {
		if err = a.checkGoFormatting(files); err != nil {
			return err
		}
		if err = a.run("cargo", "fmt", "--all", "--", "--check"); err != nil {
			return err
		}
		return a.run("rustfmt", "--edition", "2021", "--check", "crates/echo-windows/src/inline/ime_observer/dll.rs")
	}
	if len(files) > 0 {
		if err = a.run("gofmt", append([]string{"-w"}, files...)...); err != nil {
			return err
		}
	}
	if err = a.run("cargo", "fmt", "--all"); err != nil {
		return err
	}
	// This DLL entry point is compiled by the Windows adapter's build script,
	// so Cargo's crate traversal does not include it.
	return a.run("rustfmt", "--edition", "2021", "crates/echo-windows/src/inline/ime_observer/dll.rs")
}

func (a *app) checkGoFormatting(files []string) error {
	drift := make([]string, 0)
	for _, path := range files {
		source, err := os.ReadFile(path)
		if err != nil {
			return fmt.Errorf("read Go source %s: %w", path, err)
		}
		cmd := a.command("gofmt")
		cmd.Stdin = bytes.NewReader(canonicalizeLineEndings(source))
		var stderr bytes.Buffer
		cmd.Stderr = &stderr
		formatted, err := cmd.Output()
		if err != nil {
			detail := strings.TrimSpace(stderr.String())
			if detail == "" {
				return fmt.Errorf("gofmt %s: %w", path, err)
			}
			return fmt.Errorf("gofmt %s: %w: %s", path, err, detail)
		}
		if !bytes.Equal(canonicalizeLineEndings(source), formatted) {
			drift = append(drift, path)
		}
	}
	if len(drift) != 0 {
		return fmt.Errorf("gofmt would rewrite:\n%s", strings.Join(drift, "\n"))
	}
	return nil
}

func (a *app) goFiles() ([]string, error) {
	root := filepath.Join(a.root, "tools", "echo")
	var files []string
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".go") {
			files = append(files, path)
		}
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("list Echo Go sources: %w", err)
	}
	sort.Strings(files)
	return files, nil
}

func (a *app) runGoChecks() error {
	if err := a.run("go", "-C", "tools/echo", "test", "./..."); err != nil {
		return err
	}
	return a.run("go", "-C", "tools/echo", "vet", "./...")
}

func (a *app) install() error {
	if err := requireGoVersion(); err != nil {
		return err
	}
	return a.run("cargo", "fetch", "--locked")
}

func (a *app) build(release bool) error {
	args := []string{"build", "-p", "echo-desktop", "--locked", "--no-default-features"}
	if release {
		args = append(args, "--release")
	}
	return a.run("cargo", args...)
}

func (a *app) dev() error {
	return a.run("cargo", "run", "-p", "echo-desktop", "--locked", "--no-default-features")
}

func (a *app) smoke() error {
	if os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("native startup changes desktop state; set ECHO_WINDOWS_ACCEPTANCE=1")
	}
	return a.runNativeGate("smoke")
}

func (a *app) acceptanceCommand(args []string) error {
	if len(args) != 1 || (args[0] != "clipboard" && args[0] != "quick-insert" && args[0] != "ui") {
		return fmt.Errorf("acceptance requires exactly clipboard, quick-insert, or ui")
	}
	if os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("native acceptance is separately authorized; set ECHO_WINDOWS_ACCEPTANCE=1 to run it")
	}
	return a.runAcceptance(args[0])
}

func (a *app) runAcceptance(owner string) error { return a.runNativeGate(owner) }

func (a *app) packageCommand(dirOnly bool) error {
	if err := a.build(true); err != nil {
		return err
	}
	if dirOnly {
		fmt.Fprintln(a.out, a.desktopExecutable(true))
		return nil
	}
	return a.run("pwsh", "-NoProfile", "-File", filepath.Join(a.root, "tools", "packaging", "Package.ps1"), "-Root", a.root, "-Executable", a.desktopExecutable(true))
}

func (a *app) releaseCandidate() error {
	status, err := a.gitOutput("status", "--porcelain")
	if err != nil {
		return err
	}
	if strings.TrimSpace(string(status)) != "" {
		return fmt.Errorf("release-candidate requires a clean worktree")
	}
	if err := a.verifyProfile(ValidationCI); err != nil {
		return err
	}
	if err := a.packageCommand(false); err != nil {
		return err
	}
	manifestDir := filepath.Join(a.root, ".local", "echo")
	if err := os.MkdirAll(manifestDir, 0o755); err != nil {
		return err
	}
	sha, err := a.gitOutput("rev-parse", "HEAD")
	if err != nil {
		return err
	}
	manifest := map[string]string{
		"commit":     strings.TrimSpace(string(sha)),
		"profile":    string(ValidationCI),
		"executable": relativeToRoot(a.root, a.desktopExecutable(true)),
		"created_at": time.Now().UTC().Format(time.RFC3339Nano),
	}
	encoded, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(manifestDir, "release-candidate.json"), append(encoded, '\n'), 0o644)
}

func (a *app) clean() error {
	for _, path := range ownedCleanPaths(a.root) {
		if err := os.RemoveAll(path); err != nil {
			return fmt.Errorf("clean %s: %w", relativeToRoot(a.root, path), err)
		}
		fmt.Fprintf(a.out, "removed %s\n", relativeToRoot(a.root, path))
	}
	return nil
}

func ownedCleanPaths(root string) []string {
	return []string{
		filepath.Join(root, "target", "echo"),
		filepath.Join(root, ".local", "echo", "run"),
		filepath.Join(root, "tools", "echo", ".local"),
	}
}

func (a *app) desktopExecutable(release bool) string {
	profile := "debug"
	if release {
		profile = "release"
	}
	name := "echo-desktop"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	return filepath.Join(a.root, "target", profile, name)
}

func (a *app) command(name string, args ...string) *exec.Cmd {
	cmd := exec.Command(name, args...)
	cmd.Dir = a.root
	return cmd
}

func (a *app) run(name string, args ...string) error {
	if a.runOverride != nil {
		return a.runOverride(name, args...)
	}
	cmd := a.command(name, args...)
	cmd.Stdout = a.out
	cmd.Stderr = a.errOut
	cmd.Stdin = os.Stdin
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%s: %w", commandString(name, args...), err)
	}
	return nil
}

func (a *app) runWithEnv(env map[string]string, name string, args ...string) error {
	cmd := a.command(name, args...)
	cmd.Env = mergeEnv(nil, env)
	cmd.Stdout = a.out
	cmd.Stderr = a.errOut
	cmd.Stdin = os.Stdin
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%s: %w", commandString(name, args...), err)
	}
	return nil
}

func (a *app) runCapture(name string, args ...string) (string, error) {
	cmd := a.command(name, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := cmd.Run(); err != nil {
		detail := strings.TrimSpace(stderr.String())
		if detail == "" {
			detail = strings.TrimSpace(stdout.String())
		}
		return "", fmt.Errorf("%s: %w: %s", commandString(name, args...), err, detail)
	}
	return stdout.String(), nil
}

func (a *app) gitOutput(args ...string) ([]byte, error) {
	output, err := a.runCapture("git", args...)
	return []byte(output), err
}

func mergeEnv(base map[string]string, overrides map[string]string) []string {
	values := map[string]string{}
	for _, entry := range os.Environ() {
		key, value, found := strings.Cut(entry, "=")
		if found {
			values[key] = value
		}
	}
	for key, value := range base {
		values[key] = value
	}
	for key, value := range overrides {
		values[key] = value
	}
	result := make([]string, 0, len(values))
	for key, value := range values {
		result = append(result, key+"="+value)
	}
	sort.Strings(result)
	return result
}

func commandString(name string, args ...string) string {
	parts := append([]string{name}, args...)
	for index, part := range parts {
		if strings.ContainsAny(part, " \t\"") {
			parts[index] = fmt.Sprintf("%q", part)
		}
	}
	return strings.Join(parts, " ")
}

func stopOwnedProcess(cmd *exec.Cmd, done <-chan error) error {
	if cmd == nil || cmd.Process == nil || cmd.ProcessState != nil {
		return nil
	}
	if runtime.GOOS == "windows" {
		_ = exec.Command("taskkill.exe", "/PID", fmt.Sprint(cmd.Process.Pid), "/T", "/F").Run()
	} else {
		_ = cmd.Process.Signal(syscall.SIGTERM)
	}
	select {
	case <-done:
		// The process is expected to report a non-zero status when taskkill
		// terminates an owned test process tree. Ownership was already
		// established by the command we started, so that status is not a
		// validation failure.
		return nil
	case <-time.After(15 * time.Second):
		_ = cmd.Process.Kill()
		return fmt.Errorf("owned Echo process did not stop within 15 seconds")
	}
}

func relativeToRoot(root, path string) string {
	relative, err := filepath.Rel(root, path)
	if err != nil {
		return path
	}
	return filepath.ToSlash(relative)
}
