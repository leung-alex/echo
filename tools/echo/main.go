package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
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
      Install the pinned frontend and Tauri development dependencies.
  echo.cmd format [--check]
      Format or check Go, Rust, frontend, and test files.
  echo.cmd verify [--changed-from <sha>] [--profile <developer|ci>] [--explain]
      Run full verification, or verify only owners affected by Git changes.
  echo.cmd verify clipboard
      Run the focused Clipboard ownership gates.
  echo.cmd verify quick-insert
      Run the focused Quick Insert ownership gates.
  echo.cmd bindings [--check]
      Generate or check the checked-in TypeScript transport bindings.
  echo.cmd self-check
      Validate repository automation and independence invariants.
  echo.cmd build [--release]
      Build the frontend and Echo desktop executable.
  echo.cmd dev
      Start the Echo Tauri development application.
  echo.cmd smoke
      Start Echo with isolated data and verify its local bootstrap.
  echo.cmd acceptance <clipboard|quick-insert>
      Run separately authorized native Windows acceptance.
  echo.cmd package [--dir]
      Build the NSIS package, or only the unpackaged release executable.
  echo.cmd release-candidate
      Verify clean inputs and produce a local release candidate manifest.
  echo.cmd sync [--all | --branch <name>]
      Show repository and worktree status without mutating Git.
  echo.cmd clean
      Remove only Echo-owned generated bootstrap and run state.
`

type app struct {
	root   string
	out    io.Writer
	errOut io.Writer
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
	return &app{root: root, out: os.Stdout, errOut: os.Stderr}, nil
}

func (a *app) dispatch(args []string) error {
	if len(args) == 0 || args[0] == "help" || args[0] == "-h" || args[0] == "--help" {
		_, _ = io.WriteString(a.out, helpText)
		return nil
	}
	switch args[0] {
	case "install":
		return noArgs(args[1:], a.install)
	case "format":
		fs := newFlags("format", a.errOut)
		check := fs.Bool("check", false, "check formatting without edits")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		return a.format(*check)
	case "verify":
		return a.verifyCommand(args[1:])
	case "bindings":
		fs := newFlags("bindings", a.errOut)
		check := fs.Bool("check", false, "check bindings without writing")
		if err := parseFlags(fs, args[1:]); err != nil {
			return err
		}
		if *check {
			return a.checkGeneratedBindings()
		}
		return a.writeGeneratedBindings()
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
	case "clipboard", "quick-insert":
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
	if err := a.selfCheck(); err != nil {
		return err
	}
	if err := a.format(true); err != nil {
		return err
	}
	if err := a.runGoChecks(); err != nil {
		return err
	}
	switch scope {
	case "clipboard":
		if err := a.run("cargo", "test", "-p", "echo-engine"); err != nil {
			return err
		}
		if err := a.run("cargo", "test", "-p", "echo-storage"); err != nil {
			return err
		}
		return a.run("cargo", "check", "-p", "echo-desktop")
	case "quick-insert":
		if err := a.run("cargo", "test", "-p", "echo-engine"); err != nil {
			return err
		}
		if err := a.run("cargo", "test", "-p", "echo-activation"); err != nil {
			return err
		}
		if err := a.run("pnpm", "--dir", "apps/ui", "test"); err != nil {
			return err
		}
		return a.run("pnpm", "--dir", "apps/ui", "build")
	default:
		return fmt.Errorf("unknown verification scope %q", scope)
	}
}

func (a *app) verifyProfile(profile ValidationProfile) error {
	if err := a.selfCheck(); err != nil {
		return err
	}
	if err := a.format(true); err != nil {
		return err
	}
	if err := a.runGoChecks(); err != nil {
		return err
	}
	if err := a.run("cargo", "test", "--workspace", "--locked"); err != nil {
		return err
	}
	if err := a.run("pnpm", "--dir", "apps/ui", "test"); err != nil {
		return err
	}
	if err := a.run("pnpm", "--dir", "apps/ui", "build"); err != nil {
		return err
	}
	if profile == ValidationCI {
		if err := a.build(true); err != nil {
			return err
		}
	}
	return nil
}

func (a *app) runOwnerPlan(plan OwnerPlan, profile ValidationProfile) error {
	if plan.Full {
		return a.verifyProfile(profile)
	}
	if err := a.selfCheck(); err != nil {
		return err
	}
	if err := a.format(true); err != nil {
		return err
	}
	if err := a.runGoChecks(); err != nil {
		return err
	}
	needFrontend := false
	needDesktop := false
	packages := map[string]bool{}
	for _, owner := range plan.Owners {
		switch owner {
		case "clipboard", "engine":
			packages["echo-engine"], packages["echo-storage"] = true, true
		case "storage", "migration":
			packages["echo-storage"] = true
		case "library":
			packages["echo-engine"] = true
		case "quick-insert":
			packages["echo-engine"] = true
			needFrontend = true
		case "frontend":
			needFrontend = true
		case "desktop", "activation":
			needDesktop = true
		case "tests":
			needFrontend = true
			needDesktop = true
			for _, packageName := range []string{"echo-engine", "echo-storage", "echo-windows", "echo-activation"} {
				packages[packageName] = true
			}
		case "tooling":
			// self-check and Go checks above are the tooling gates.
		}
	}
	packageNames := make([]string, 0, len(packages))
	for packageName := range packages {
		packageNames = append(packageNames, packageName)
	}
	sort.Strings(packageNames)
	for _, packageName := range packageNames {
		if err := a.run("cargo", "test", "-p", packageName); err != nil {
			return err
		}
	}
	if needFrontend {
		if err := a.run("pnpm", "--dir", "apps/ui", "test"); err != nil {
			return err
		}
		if profile == ValidationCI || contains(plan.Owners, "frontend") {
			if err := a.run("pnpm", "--dir", "apps/ui", "build"); err != nil {
				return err
			}
		}
	}
	if needDesktop {
		if err := a.run("cargo", "check", "-p", "echo-desktop"); err != nil {
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
	for _, relative := range []string{
		"echo.cmd",
		"Cargo.toml",
		"pnpm-workspace.yaml",
		"package.json",
		"apps/ui/package.json",
		"apps/desktop/tauri.conf.json",
		"apps/desktop/icons/icon.ico",
		"tools/echo/go.mod",
		"tools/echo/fixture/main_windows.go",
		"docs/TEST_OWNERSHIP_MAP.md",
		"docs/VISUAL_PARITY_DEVIATIONS.md",
		"docs/P08_VISUAL_HANDOFF.md",
	} {
		if !fileExists(filepath.Join(a.root, filepath.FromSlash(relative))) {
			return fmt.Errorf("self-check expected file is missing: %s", relative)
		}
	}
	if fileExists(filepath.Join(a.root, ".gitmodules")) {
		return fmt.Errorf("self-check rejects Git submodules")
	}
	if err := a.checkManifestIndependence(); err != nil {
		return err
	}
	if err := a.checkFrontendPresentation(); err != nil {
		return err
	}
	metadata, err := a.runCapture("cargo", "metadata", "--no-deps", "--locked", "--format-version", "1")
	if err != nil {
		return err
	}
	if containsForbiddenDependency(metadata) {
		return fmt.Errorf("self-check found a sibling dependency in cargo metadata")
	}
	if err := a.checkGeneratedIgnore("apps/desktop/gen/schemas/desktop-schema.json"); err != nil {
		return err
	}
	if err := a.checkGeneratedBindings(); err != nil {
		return err
	}
	if err := a.checkNativeFixtureSources(); err != nil {
		return err
	}
	content, err := os.ReadFile(filepath.Join(a.root, "docs", "TEST_OWNERSHIP_MAP.md"))
	if err != nil {
		return fmt.Errorf("read test ownership map: %w", err)
	}
	for _, required := range []string{"clipboard.spec.ts", "quick-insert.spec.ts", "Native acceptance"} {
		if !bytes.Contains(content, []byte(required)) {
			return fmt.Errorf("test ownership map is missing %q", required)
		}
	}
	return nil
}

func (a *app) checkNativeFixtureSources() error {
	for _, relative := range []string{
		"tests/e2e/native-fixture.ts",
		"tools/echo/fixture/main_windows.go",
		"tools/echo/fixture/target_windows.go",
	} {
		if !fileExists(filepath.Join(a.root, filepath.FromSlash(relative))) {
			return fmt.Errorf("native fixture source is missing: %s", relative)
		}
	}
	var powershellFiles []string
	err := filepath.WalkDir(filepath.Join(a.root, "tests", "e2e"), func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if !entry.IsDir() && strings.EqualFold(filepath.Ext(path), ".ps1") {
			powershellFiles = append(powershellFiles, relativeToRoot(a.root, path))
		}
		return nil
	})
	if err != nil {
		return fmt.Errorf("scan native fixture sources: %w", err)
	}
	if len(powershellFiles) != 0 {
		return fmt.Errorf("legacy .ps1 native fixture remains: %s", strings.Join(powershellFiles, ", "))
	}
	return nil
}

func (a *app) checkFrontendPresentation() error {
	root := filepath.Join(a.root, "apps", "ui", "src")
	defined := map[string]bool{}
	used := map[string]bool{}
	err := filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			return nil
		}
		ext := strings.ToLower(filepath.Ext(path))
		if ext != ".ts" && ext != ".tsx" && ext != ".css" {
			return nil
		}
		content, err := os.ReadFile(path)
		if err != nil {
			return fmt.Errorf("read frontend source %s: %w", relativeToRoot(a.root, path), err)
		}
		textContent := string(content)
		lower := strings.ToLower(textContent)
		if strings.Contains(lower, "innerhtml") {
			return fmt.Errorf("temporary innerHTML presentation remains in %s", relativeToRoot(a.root, path))
		}
		if strings.Contains(lower, "culsans") {
			return fmt.Errorf("Culsans UI/source reference remains in %s", relativeToRoot(a.root, path))
		}
		if ext != ".css" {
			return nil
		}
		for _, occurrence := range echoTokenOccurrences(textContent) {
			if occurrence.definition {
				defined[occurrence.token] = true
			} else {
				used[occurrence.token] = true
			}
		}
		return nil
	})
	if err != nil {
		return err
	}
	for token := range used {
		if !defined[token] {
			return fmt.Errorf("unresolved Echo UI token: %s", token)
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
	files := []string{
		"Cargo.toml",
		"Cargo.lock",
		"pnpm-workspace.yaml",
		"pnpm-lock.yaml",
		"package.json",
		"apps/ui/package.json",
		"apps/desktop/Cargo.toml",
		"apps/desktop/tauri.conf.json",
		"echo.cmd",
		"tools/echo/go.mod",
	}
	err := filepath.WalkDir(a.root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.IsDir() {
			name := entry.Name()
			if name == ".git" || name == "target" || name == "node_modules" || name == ".local" || name == "dist" {
				return filepath.SkipDir
			}
			return nil
		}
		name := filepath.Base(path)
		if name == "Cargo.toml" || name == "package.json" || name == "pnpm-workspace.yaml" || name == ".gitmodules" || name == "echo.cmd" || name == "go.mod" {
			files = append(files, path)
		}
		return nil
	})
	if err != nil {
		return fmt.Errorf("scan Echo manifests: %w", err)
	}
	seen := map[string]bool{}
	for _, path := range files {
		absolute, err := filepath.Abs(path)
		if err != nil {
			return err
		}
		if seen[strings.ToLower(absolute)] {
			continue
		}
		seen[strings.ToLower(absolute)] = true
		content, err := os.ReadFile(absolute)
		if err != nil {
			return fmt.Errorf("read manifest %s: %w", path, err)
		}
		if containsForbiddenDependency(string(content)) {
			return fmt.Errorf("sibling dependency found in %s", relativeToRoot(a.root, absolute))
		}
	}
	return nil
}

func containsForbiddenDependency(content string) bool {
	lower := strings.ToLower(strings.ReplaceAll(content, "\\", "/"))
	for _, forbidden := range []string{
		"path = \"../",
		"path='../",
		"file:../",
		"file:..//",
		"../culsans",
		"d:/project/culsans",
		"culsans-storage",
		"culsans-runtime",
		"culsans-platform",
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
	goFiles, err := a.goFiles()
	if err != nil {
		return err
	}
	if check {
		if len(goFiles) != 0 {
			output, err := a.runCapture("gofmt", append([]string{"-l"}, goFiles...)...)
			if err != nil {
				return err
			}
			if strings.TrimSpace(output) != "" {
				return fmt.Errorf("gofmt would rewrite:\n%s", output)
			}
		}
		if err := a.run("cargo", "fmt", "--all", "--", "--check"); err != nil {
			return err
		}
		args := append([]string{"exec", "prettier", "--check"}, prettierTargets()...)
		return a.run("pnpm", args...)
	}
	if len(goFiles) != 0 {
		if err := a.run("gofmt", append([]string{"-w"}, goFiles...)...); err != nil {
			return err
		}
	}
	if err := a.run("cargo", "fmt", "--all"); err != nil {
		return err
	}
	args := append([]string{"exec", "prettier", "--write"}, prettierTargets()...)
	return a.run("pnpm", args...)
}

func prettierTargets() []string {
	return []string{
		"apps/ui/**/*.{ts,tsx,css,json,html}",
		"tests/**/*.ts",
		"package.json",
		"pnpm-workspace.yaml",
	}
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
	return a.run("pnpm", "install", "--frozen-lockfile")
}

func (a *app) build(release bool) error {
	if err := a.run("pnpm", "--dir", "apps/ui", "build"); err != nil {
		return err
	}
	args := []string{"build", "-p", "echo-desktop", "--locked"}
	if release {
		args = append(args, "--release", "--features", "custom-protocol")
	}
	return a.run("cargo", args...)
}

func (a *app) dev() error {
	return a.run("pnpm", "exec", "tauri", "dev", "--config", "apps/desktop/tauri.conf.json")
}

func (a *app) smoke() error {
	if err := a.build(false); err != nil {
		return err
	}
	runRoot := filepath.Join(a.root, ".local", "echo")
	if err := os.MkdirAll(runRoot, 0o755); err != nil {
		return fmt.Errorf("create smoke run root: %w", err)
	}
	dataDir, err := os.MkdirTemp(runRoot, "smoke-")
	if err != nil {
		return fmt.Errorf("create smoke data directory: %w", err)
	}
	defer os.RemoveAll(dataDir)
	exe := a.desktopExecutable(false)
	if !fileExists(exe) {
		return fmt.Errorf("Echo desktop executable is missing: %s", exe)
	}
	logPath := filepath.Join(dataDir, "smoke.log")
	logFile, err := os.Create(logPath)
	if err != nil {
		return fmt.Errorf("create smoke log: %w", err)
	}
	cmd := a.command(exe)
	cmd.Env = mergeEnv(nil, map[string]string{"ECHO_DATA_DIR": filepath.Join(dataDir, "data")})
	cmd.Stdout, cmd.Stderr = logFile, logFile
	if err := cmd.Start(); err != nil {
		_ = logFile.Close()
		return fmt.Errorf("start Echo smoke process: %w", err)
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case err := <-done:
		_ = logFile.Close()
		return fmt.Errorf("Echo exited during smoke: %w; log=%s", err, logPath)
	case <-time.After(4 * time.Second):
	}
	dataPath := filepath.Join(dataDir, "data")
	if !fileExists(filepath.Join(dataPath, "echo.sqlite3")) || !directoryExists(filepath.Join(dataPath, "blobs")) {
		_ = stopOwnedProcess(cmd, done)
		_ = logFile.Close()
		return fmt.Errorf("Echo smoke did not create its database/blob root; log=%s", logPath)
	}
	if err := stopOwnedProcess(cmd, done); err != nil {
		_ = logFile.Close()
		return err
	}
	_ = logFile.Close()
	fmt.Fprintf(a.out, "Echo smoke passed; isolated data=%s\n", dataPath)
	return nil
}

func (a *app) acceptanceCommand(args []string) error {
	if len(args) != 1 || (args[0] != "clipboard" && args[0] != "quick-insert") {
		return fmt.Errorf("acceptance requires exactly clipboard or quick-insert")
	}
	if os.Getenv("ECHO_WINDOWS_ACCEPTANCE") != "1" {
		return fmt.Errorf("native acceptance is separately authorized; set ECHO_WINDOWS_ACCEPTANCE=1 to run it")
	}
	return a.runAcceptance(args[0])
}

func (a *app) runAcceptance(owner string) error {
	if runtime.GOOS != "windows" {
		return fmt.Errorf("native acceptance requires Windows")
	}
	if err := a.build(true); err != nil {
		return err
	}
	runRoot := filepath.Join(a.root, ".local", "echo")
	if err := os.MkdirAll(runRoot, 0o755); err != nil {
		return fmt.Errorf("create acceptance root: %w", err)
	}
	runDir, err := os.MkdirTemp(runRoot, "acceptance-")
	if err != nil {
		return fmt.Errorf("create acceptance run: %w", err)
	}
	defer os.RemoveAll(runDir)
	fixtureExecutable, err := a.buildNativeFixture(runDir)
	if err != nil {
		return err
	}
	dataDir := filepath.Join(runDir, "data")
	webviewDir := filepath.Join(runDir, "webview2")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return err
	}
	if err := os.MkdirAll(webviewDir, 0o755); err != nil {
		return err
	}
	port, err := freeLoopbackPort()
	if err != nil {
		return err
	}
	exe := a.desktopExecutable(true)
	logPath := filepath.Join(runDir, "echo.log")
	logFile, err := os.Create(logPath)
	if err != nil {
		return fmt.Errorf("create acceptance log: %w", err)
	}
	env := map[string]string{
		"ECHO_WINDOWS_ACCEPTANCE":               "1",
		"ECHO_ACCEPTANCE_CDP_PORT":              fmt.Sprint(port),
		"ECHO_ACCEPTANCE_EXE":                   exe,
		"ECHO_ACCEPTANCE_FIXTURE_EXE":           fixtureExecutable,
		"ECHO_ACCEPTANCE_RUN_ROOT":              runDir,
		"ECHO_DATA_DIR":                         dataDir,
		"WEBVIEW2_USER_DATA_FOLDER":             webviewDir,
		"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--remote-debugging-port=" + fmt.Sprint(port),
	}
	if legacy := os.Getenv("ECHO_LEGACY_DATA_DIR"); legacy != "" {
		env["ECHO_LEGACY_DATA_DIR"] = legacy
	}
	cmd := a.command(exe)
	cmd.Env = mergeEnv(nil, env)
	cmd.Stdout, cmd.Stderr = logFile, logFile
	if err := cmd.Start(); err != nil {
		_ = logFile.Close()
		return fmt.Errorf("start Echo acceptance process: %w", err)
	}
	env["ECHO_ACCEPTANCE_PID"] = fmt.Sprint(cmd.Process.Pid)
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	if err := waitForCDP(port, done); err != nil {
		_ = stopOwnedProcess(cmd, done)
		_ = logFile.Close()
		return fmt.Errorf("Echo acceptance did not become ready: %w; log=%s", err, logPath)
	}
	playwrightSpec := filepath.ToSlash(filepath.Join("tests", "e2e", owner+".spec.ts"))
	args := []string{"exec", "playwright", "test", "--config", "tests/e2e/playwright.config.ts", playwrightSpec}
	result := a.runWithEnv(env, "pnpm", args...)
	stopErr := stopOwnedProcess(cmd, done)
	_ = logFile.Close()
	if result != nil {
		return fmt.Errorf("Echo %s acceptance failed: %w; log=%s", owner, result, logPath)
	}
	if stopErr != nil {
		return stopErr
	}
	return nil
}

func (a *app) buildNativeFixture(outputDir string) (string, error) {
	if runtime.GOOS != "windows" {
		return "", fmt.Errorf("native fixture requires Windows")
	}
	output := filepath.Join(outputDir, "echo-native-fixture.exe")
	if err := a.run("go", "-C", "tools/echo", "build", "-trimpath", "-o", output, "./fixture"); err != nil {
		return "", fmt.Errorf("build Echo native fixture: %w", err)
	}
	if !fileExists(output) {
		return "", fmt.Errorf("native fixture executable is missing: %s", output)
	}
	return output, nil
}

func (a *app) packageCommand(dirOnly bool) error {
	if dirOnly {
		if err := a.build(true); err != nil {
			return err
		}
		fmt.Fprintln(a.out, a.desktopExecutable(true))
		return nil
	}
	if err := a.run("pnpm", "exec", "tauri", "build", "--config", "apps/desktop/tauri.conf.json"); err != nil {
		return err
	}
	return nil
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
	if err := a.packageCommand(true); err != nil {
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

func (a *app) syncCommand(args []string) error {
	fs := newFlags("sync", a.errOut)
	all := fs.Bool("all", false, "show all worktrees")
	branch := fs.String("branch", "", "show the selected worktree branch")
	if err := parseFlags(fs, args); err != nil {
		return err
	}
	if *all && *branch != "" {
		return fmt.Errorf("sync accepts either --all or --branch, not both")
	}
	if err := a.run("git", "status", "--short", "--branch"); err != nil {
		return err
	}
	worktrees, err := a.gitOutput("worktree", "list", "--porcelain")
	if err != nil {
		return err
	}
	if *branch == "" && !*all {
		fmt.Fprintln(a.out, string(worktrees))
		return nil
	}
	if *all {
		fmt.Fprintln(a.out, string(worktrees))
		return nil
	}
	for _, block := range strings.Split(strings.TrimSpace(string(worktrees)), "\n\n") {
		if strings.Contains(block, "branch refs/heads/"+*branch) {
			fmt.Fprintln(a.out, block)
		}
	}
	return nil
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

func freeLoopbackPort() (int, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, fmt.Errorf("reserve a loopback port: %w", err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port, nil
}

func waitForCDP(port int, done <-chan error) error {
	client := &http.Client{Timeout: 500 * time.Millisecond}
	url := fmt.Sprintf("http://127.0.0.1:%d/json/version", port)
	deadline := time.Now().Add(60 * time.Second)
	for time.Now().Before(deadline) {
		select {
		case err := <-done:
			return fmt.Errorf("Echo exited before CDP readiness: %w", err)
		default:
		}
		request, err := http.NewRequestWithContext(context.Background(), http.MethodGet, url, nil)
		if err == nil {
			response, requestErr := client.Do(request)
			if requestErr == nil {
				_ = response.Body.Close()
				if response.StatusCode >= 200 && response.StatusCode < 300 {
					return nil
				}
			}
		}
		time.Sleep(250 * time.Millisecond)
	}
	return fmt.Errorf("CDP endpoint %s did not become ready", url)
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
		// terminates the WebView2 descendant tree. Ownership was already
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
