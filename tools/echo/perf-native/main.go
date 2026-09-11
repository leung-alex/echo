// Command perf-native manages evidence before any product migration is allowed.
// It never starts Echo, changes the clipboard, kills a process, or mutates Git.
package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"image/png"
	"io"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"time"
)

const sourceSHA = "0dc699e42d8d667e502938d71e33216f92513e5a"
const maxJSONBytes = 4 * 1024 * 1024

const usage = `Echo native performance evidence tool (M0 preparation only)

  go -C tools/echo run ./perf-native preflight --repo <repo> --out <new-file>
  go -C tools/echo run ./perf-native inventory --runtime <dir> --out <new-file> [--installer <file>]
  go -C tools/echo run ./perf-native check-g0 --bundle <evidence-directory>

preflight checks the original product source and the local execution prerequisites.
It does NOT build, measure or accept Echo. Exit 2 means blocked or invalid evidence.
check-g0 checks evidence integrity and coverage, NOT screenshot quality or correctness.
Evidence must stay outside the repository. Existing evidence is never overwritten.
`

type artifact struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}

type gate struct {
	Status string   `json:"status"`
	Log    artifact `json:"log"`
}

type measurement struct {
	RunID string   `json:"run_id"`
	Value *float64 `json:"value"`
	Raw   artifact `json:"raw"`
}

type metric struct {
	Scenario     string        `json:"scenario"`
	Name         string        `json:"metric"`
	Unit         string        `json:"unit"`
	SamplingUnit string        `json:"sampling_unit"`
	Samples      []measurement `json:"samples"`
}

type environmentEvidence struct {
	OS                  string   `json:"os"`
	WindowsBuild        string   `json:"windows_build"`
	InteractiveSession  bool     `json:"interactive_session"`
	MachineID           string   `json:"machine_id"`
	ComparisonProfileID string   `json:"comparison_profile_id"`
	Details             artifact `json:"details"`
}

type fixtureEvidence struct {
	History    int      `json:"history"`
	SavedItems int      `json:"saved_items"`
	Images     int      `json:"images"`
	Snapshot   artifact `json:"snapshot"`
}

type baselineEvidence struct {
	Schema             string                     `json:"schema"`
	Status             string                     `json:"status"`
	Variant            string                     `json:"variant"`
	SourceSHA          string                     `json:"source_sha"`
	BuildProfile       string                     `json:"build_profile"`
	WindowCount        int                        `json:"window_count"`
	Clock              string                     `json:"clock"`
	ClockFrequency     int64                      `json:"clock_frequency"`
	ReadinessEndpoint  string                     `json:"readiness_endpoint"`
	Environment        environmentEvidence        `json:"environment"`
	Binary             artifact                   `json:"binary"`
	Inventory          artifact                   `json:"inventory"`
	ProcessSetVerified bool                       `json:"process_set_verified"`
	ProcessOwnership   artifact                   `json:"process_ownership"`
	ObserverOverhead   artifact                   `json:"observer_overhead"`
	Fixtures           map[string]fixtureEvidence `json:"fixtures"`
	Gates              map[string]gate            `json:"gates"`
	Screenshots        map[string]artifact        `json:"screenshots"`
	Metrics            []metric                   `json:"metrics"`
}

type preflightReport struct {
	Schema            string          `json:"schema"`
	Status            string          `json:"status"`
	G0                string          `json:"g0"`
	CreatedUTC        string          `json:"created_at_utc"`
	HostOS            string          `json:"host_os"`
	HostArch          string          `json:"host_arch"`
	GoVersion         string          `json:"go_version"`
	ExpectedSourceSHA string          `json:"expected_source_sha"`
	CheckoutSHA       string          `json:"checkout_sha"`
	Branch            string          `json:"branch"`
	Dirty             bool            `json:"dirty"`
	ProductChanges    []string        `json:"product_changes"`
	Tools             map[string]bool `json:"tools"`
	Blockers          []string        `json:"blockers"`
}

type fileInventory struct {
	Path   string `json:"path"`
	Bytes  int64  `json:"bytes"`
	SHA256 string `json:"sha256"`
}

type inventoryReport struct {
	Schema       string          `json:"schema"`
	Status       string          `json:"status"`
	RuntimeBytes int64           `json:"runtime_bytes"`
	Files        []fileInventory `json:"files"`
	Installer    *fileInventory  `json:"installer,omitempty"`
	Note         string          `json:"note"`
}

func main() {
	if err := command(os.Args[1:], os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "ERROR:", err)
		os.Exit(2)
	}
}

func command(args []string, out io.Writer) error {
	if len(args) == 0 || args[0] == "help" || args[0] == "--help" {
		_, err := io.WriteString(out, usage)
		return err
	}
	fs := flag.NewFlagSet(args[0], flag.ContinueOnError)
	fs.SetOutput(out)
	switch args[0] {
	case "preflight":
		repo := fs.String("repo", ".", "repository root")
		output := fs.String("out", "", "new evidence file outside repository")
		if err := parse(fs, args[1:]); err != nil {
			return err
		}
		if *output == "" {
			return errors.New("--out is required")
		}
		root, err := filepath.Abs(*repo)
		if err != nil {
			return err
		}
		actualRoot, err := gitRead(root, "rev-parse", "--show-toplevel")
		if err != nil {
			return err
		}
		root = strings.TrimSpace(actualRoot)
		if err := outside(root, *output); err != nil {
			return err
		}
		report := preflight(root, runtime.GOOS, os.Getenv("ECHO_WINDOWS_ACCEPTANCE"))
		if err := writeNewJSON(*output, report); err != nil {
			return err
		}
		fmt.Fprintln(out, report.Status, *output)
		if len(report.Blockers) > 0 {
			return errors.New(strings.Join(report.Blockers, "; "))
		}
		return nil
	case "inventory":
		root := fs.String("runtime", "", "complete installed runtime directory")
		installer := fs.String("installer", "", "installer, counted separately")
		output := fs.String("out", "", "new evidence file outside runtime")
		if err := parse(fs, args[1:]); err != nil {
			return err
		}
		if *root == "" || *output == "" {
			return errors.New("--runtime and --out are required")
		}
		if err := outside(*root, *output); err != nil {
			return err
		}
		if *installer != "" {
			if err := outside(*root, *installer); err != nil {
				return fmt.Errorf("installer must be separate: %w", err)
			}
			a, _ := filepath.Abs(*installer)
			b, _ := filepath.Abs(*output)
			if strings.EqualFold(a, b) {
				return errors.New("output cannot replace installer")
			}
		}
		report, err := inventory(*root, *installer)
		if err != nil {
			return err
		}
		if err := writeNewJSON(*output, report); err != nil {
			return err
		}
		fmt.Fprintln(out, "INVENTORIED (not a performance or installation acceptance)", *output)
		return nil
	case "check-g0":
		bundle := fs.String("bundle", "", "directory containing g0.json and referenced evidence")
		if err := parse(fs, args[1:]); err != nil {
			return err
		}
		if *bundle == "" {
			return errors.New("--bundle is required")
		}
		if err := validateG0(*bundle); err != nil {
			return err
		}
		fmt.Fprintln(out, "G0_EVIDENCE_INTEGRITY_VALID; independent semantic/visual review still required")
		return nil
	default:
		return fmt.Errorf("unknown command %q", args[0])
	}
}

func parse(fs *flag.FlagSet, args []string) error {
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return fmt.Errorf("unexpected positional arguments: %v", fs.Args())
	}
	return nil
}

func gitRead(root string, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "git", append([]string{"-C", root}, args...)...)
	data, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("git %s failed: %w", strings.Join(args, " "), err)
	}
	return string(data), nil
}

func preparationOnly(path string) bool {
	return strings.HasPrefix(path, "tools/echo/perf-native/") ||
		strings.HasPrefix(path, "tools/perf-native/") ||
		strings.HasPrefix(path, "docs/migration/slint/")
}

func preflight(root, hostOS, authorization string) preflightReport {
	r := preflightReport{
		Schema: "echo.native.preflight.v1", Status: "BLOCKED", G0: "NOT_RUN",
		CreatedUTC: time.Now().UTC().Format(time.RFC3339Nano), HostOS: hostOS,
		HostArch: runtime.GOARCH, GoVersion: runtime.Version(), ExpectedSourceSHA: sourceSHA,
		Tools: map[string]bool{}, ProductChanges: []string{}, Blockers: []string{},
	}
	if hostOS != "windows" {
		r.Blockers = append(r.Blockers, "real Windows execution is required; Linux results cannot release G0")
	}
	if authorization != "1" {
		r.Blockers = append(r.Blockers, "ECHO_WINDOWS_ACCEPTANCE=1 is required for the later native test phase")
	}
	var err error
	r.CheckoutSHA, err = gitRead(root, "rev-parse", "--verify", "HEAD")
	if err != nil {
		r.Blockers = append(r.Blockers, err.Error())
	}
	r.CheckoutSHA = strings.TrimSpace(r.CheckoutSHA)
	r.Branch, err = gitRead(root, "branch", "--show-current")
	if err != nil {
		r.Blockers = append(r.Blockers, "cannot inspect the current Git branch")
	}
	r.Branch = strings.TrimSpace(r.Branch)
	if r.Branch == "" {
		r.Branch = "DETACHED"
	}
	status, err := gitRead(root, "status", "--porcelain", "--untracked-files=all")
	if err != nil {
		r.Blockers = append(r.Blockers, err.Error())
	}
	r.Dirty = status != ""
	if r.Dirty {
		r.Blockers = append(r.Blockers, "worktree has changes; do not reset or clean it")
	}
	changes, err := gitRead(root, "diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", sourceSHA, "HEAD", "--")
	if err != nil {
		r.Blockers = append(r.Blockers, "original baseline source commit is not available")
	} else {
		for _, path := range strings.Split(changes, "\x00") {
			if path != "" && !preparationOnly(path) {
				r.ProductChanges = append(r.ProductChanges, path)
			}
		}
	}
	if len(r.ProductChanges) > 0 {
		r.Blockers = append(r.Blockers, "product/build changes exist before A0 baseline")
	}
	for _, name := range []string{"git", "go", "cargo", "rustc", "pnpm", "pwsh"} {
		_, err := exec.LookPath(name)
		r.Tools[name] = err == nil
		if err != nil {
			r.Blockers = append(r.Blockers, "required executable not found: "+name)
		}
	}
	if len(r.Blockers) == 0 {
		r.Status = "READY_FOR_BASELINE_NOT_ACCEPTED"
	}
	return r
}

// Reject links in every existing component, including ancestors of a new output.
func noLinks(path string) error {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return err
	}
	for current := absolute; ; current = filepath.Dir(current) {
		info, statErr := os.Lstat(current)
		if statErr != nil && !os.IsNotExist(statErr) {
			return statErr
		}
		if statErr == nil && info.Mode()&os.ModeSymlink != 0 {
			return fmt.Errorf("symbolic link/reparse alias is not permitted: %s", current)
		}
		parent := filepath.Dir(current)
		if parent == current {
			break
		}
	}
	return nil
}

func outside(root, output string) error {
	for _, path := range []string{root, output} {
		if err := noLinks(path); err != nil {
			return err
		}
	}
	a, err := filepath.Abs(root)
	if err != nil {
		return err
	}
	b, err := filepath.Abs(output)
	if err != nil {
		return err
	}
	relative, err := filepath.Rel(a, b)
	if err != nil {
		// Separate Windows volumes are outside each other.
		if runtime.GOOS == "windows" && !strings.EqualFold(filepath.VolumeName(a), filepath.VolumeName(b)) {
			return nil
		}
		return err
	}
	if relative == "." || (relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator))) {
		return errors.New("evidence/output must be outside the input directory")
	}
	return nil
}

func digest(path string) (int64, string, error) {
	if err := noLinks(path); err != nil {
		return 0, "", err
	}
	info, err := os.Lstat(path)
	if err != nil {
		return 0, "", err
	}
	if !info.Mode().IsRegular() {
		return 0, "", fmt.Errorf("not a regular file: %s", path)
	}
	f, err := os.Open(path)
	if err != nil {
		return 0, "", err
	}
	defer f.Close()
	h := sha256.New()
	n, err := io.Copy(h, f)
	if err != nil {
		return 0, "", err
	}
	after, err := f.Stat()
	if err != nil {
		return 0, "", err
	}
	if n != info.Size() || after.Size() != info.Size() || !after.ModTime().Equal(info.ModTime()) {
		return 0, "", errors.New("file changed while hashing")
	}
	return n, hex.EncodeToString(h.Sum(nil)), nil
}

func inventory(root, installer string) (inventoryReport, error) {
	r := inventoryReport{Schema: "echo.native.inventory.v1", Status: "INVENTORIED", Files: []fileInventory{}, Note: "Logical file lengths; not allocated disk clusters. Installer is separate. Shared OS runtime is not included. This does not certify an installation."}
	if err := noLinks(root); err != nil {
		return r, err
	}
	info, err := os.Stat(root)
	if err != nil {
		return r, err
	}
	if !info.IsDir() {
		return r, errors.New("runtime must be a directory")
	}
	err = filepath.WalkDir(root, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if entry.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("link in runtime: %s", path)
		}
		if entry.IsDir() {
			return nil
		}
		n, hash, err := digest(path)
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		if n > math.MaxInt64-r.RuntimeBytes {
			return errors.New("inventory size overflow")
		}
		r.RuntimeBytes += n
		r.Files = append(r.Files, fileInventory{filepath.ToSlash(rel), n, hash})
		return nil
	})
	if err != nil {
		return r, err
	}
	if len(r.Files) == 0 {
		return r, errors.New("runtime contains no files")
	}
	sort.Slice(r.Files, func(i, j int) bool { return r.Files[i].Path < r.Files[j].Path })
	if installer != "" {
		n, hash, err := digest(installer)
		if err != nil {
			return r, err
		}
		r.Installer = &fileInventory{filepath.Base(installer), n, hash}
	}
	return r, nil
}

func writeNewJSON(path string, value any) error {
	if err := noLinks(path); err != nil {
		return err
	}
	encoded, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	// Do not create parents or overwrite artifacts. The owner chooses the evidence root.
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	_, writeErr := f.Write(append(encoded, '\n'))
	syncErr := f.Sync()
	closeErr := f.Close()
	return errors.Join(writeErr, syncErr, closeErr)
}

func validHash(value string, length int) bool {
	if len(value) != length || value != strings.ToLower(value) {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}

func checkedArtifact(root string, a artifact) (string, error) {
	if a.Path == "" || strings.ContainsAny(a.Path, "\\:\x00") || filepath.IsAbs(a.Path) || !validHash(a.SHA256, 64) {
		return "", errors.New("artifact requires a relative slash path and lowercase SHA-256")
	}
	for _, component := range strings.Split(a.Path, "/") {
		if component == "" || component == "." || component == ".." || strings.TrimRight(component, " .") != component || reservedName(component) {
			return "", errors.New("artifact path contains an unsafe component")
		}
	}
	path := filepath.Join(root, filepath.FromSlash(a.Path))
	n, hash, err := digest(path)
	if err != nil {
		return "", err
	}
	if n == 0 {
		return "", fmt.Errorf("empty evidence: %s", a.Path)
	}
	if hash != a.SHA256 {
		return "", fmt.Errorf("evidence hash mismatch: %s", a.Path)
	}
	return path, nil
}

// encoding/json normally accepts duplicate fields. Evidence must not have two
// conflicting statuses/values that different consumers could interpret differently.
func rejectDuplicateKeys(data []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	var visit func() error
	visit = func() error {
		token, err := decoder.Token()
		if err != nil {
			return err
		}
		delimiter, ok := token.(json.Delim)
		if !ok {
			return nil
		}
		switch delimiter {
		case '{':
			seen := map[string]bool{}
			for decoder.More() {
				token, err := decoder.Token()
				if err != nil {
					return err
				}
				key, ok := token.(string)
				if !ok {
					return errors.New("invalid JSON object key")
				}
				if seen[key] {
					return fmt.Errorf("duplicate JSON key: %s", key)
				}
				seen[key] = true
				if err := visit(); err != nil {
					return err
				}
			}
		case '[':
			for decoder.More() {
				if err := visit(); err != nil {
					return err
				}
			}
		default:
			return errors.New("unexpected JSON closing delimiter")
		}
		_, err = decoder.Token()
		return err
	}
	if err := visit(); err != nil {
		return err
	}
	if _, err := decoder.Token(); err != io.EOF {
		return errors.New("trailing JSON values are not permitted")
	}
	return nil
}

func decodeEvidence(path string, target any) error {
	if err := noLinks(path); err != nil {
		return err
	}
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, maxJSONBytes+1))
	if err != nil {
		return err
	}
	if len(data) > maxJSONBytes {
		return errors.New("evidence JSON exceeds 4 MiB")
	}
	if err := rejectDuplicateKeys(data); err != nil {
		return err
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	return decoder.Decode(target)
}

func validateG0(root string) error {
	var e baselineEvidence
	if err := decodeEvidence(filepath.Join(root, "g0.json"), &e); err != nil {
		return err
	}
	if e.Schema != "echo.native.g0.v1" || e.Status != "MEASURED" || e.Variant != "A0" {
		return errors.New("G0 requires measured A0 evidence, not a template or candidate")
	}
	if e.SourceSHA != sourceSHA || e.BuildProfile != "release" || e.WindowCount != 2 {
		return errors.New("source, release profile, or dual-window parity differs from the original baseline")
	}
	if e.Clock != "QPC" || e.ClockFrequency <= 0 || e.ReadinessEndpoint != "external-semantic-ui-ready" {
		return errors.New("HWND/show/WaitForInputIdle is not semantic UI readiness")
	}
	if e.Environment.OS != "windows" || !e.Environment.InteractiveSession || e.Environment.WindowsBuild == "" || e.Environment.MachineID == "" || e.Environment.ComparisonProfileID == "" {
		return errors.New("matched interactive Windows environment evidence is missing")
	}
	if !e.ProcessSetVerified {
		return errors.New("complete Echo/WebView2 process ownership is not verified")
	}
	for name, ref := range map[string]artifact{
		"environment": e.Environment.Details, "binary": e.Binary, "inventory": e.Inventory,
		"process ownership": e.ProcessOwnership, "observer overhead": e.ObserverOverhead,
	} {
		if _, err := checkedArtifact(root, ref); err != nil {
			return fmt.Errorf("%s: %w", name, err)
		}
	}
	for _, name := range []string{"self-check", "format", "bindings", "verify", "smoke", "storage-perf", "acceptance-clipboard", "acceptance-quick-insert", "visual-parity"} {
		g, ok := e.Gates[name]
		if !ok || g.Status != "PASS" {
			return fmt.Errorf("required baseline gate has not passed: %s", name)
		}
		if _, err := checkedArtifact(root, g.Log); err != nil {
			return fmt.Errorf("gate %s: %w", name, err)
		}
	}
	d1, ok := e.Fixtures["D1"]
	if !ok || d1.History != 5000 || d1.SavedItems != 200 {
		return errors.New("D1 must contain 5000 history and 200 saved items")
	}
	d2, ok := e.Fixtures["D2"]
	if !ok || d2.History != 200 || d2.Images != 50 {
		return errors.New("D2 must contain 200 history entries including 50 images")
	}
	for _, fixture := range []fixtureEvidence{d1, d2} {
		if _, err := checkedArtifact(root, fixture.Snapshot); err != nil {
			return fmt.Errorf("fixture: %w", err)
		}
	}
	screenshotPaths, screenshotHashes := map[string]bool{}, map[string]bool{}
	for _, name := range []string{"history-light", "history-dark", "favorites", "hover", "settings", "mixed-images"} {
		ref, ok := e.Screenshots[name]
		if !ok {
			return fmt.Errorf("missing screenshot: %s", name)
		}
		if screenshotPaths[ref.Path] || screenshotHashes[ref.SHA256] {
			return errors.New("each screenshot state requires its own capture")
		}
		screenshotPaths[ref.Path], screenshotHashes[ref.SHA256] = true, true
		path, err := checkedArtifact(root, ref)
		if err != nil {
			return err
		}
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		config, decodeErr := png.DecodeConfig(io.LimitReader(f, maxJSONBytes))
		_ = f.Close()
		if decodeErr != nil || config.Width < 1 || config.Height < 1 || config.Width > 32768 || config.Height > 32768 {
			return fmt.Errorf("invalid PNG screenshot metadata: %s", name)
		}
	}

	required := map[string]struct {
		unit  string
		count int
	}{
		"process-start-d1/ui_ready":                  {"ms", 30},
		"hot-show-d1/ui_ready":                       {"ms", 50},
		"search-d1/end_to_end":                       {"ms", 50},
		"hidden-steady-d1/private_bytes":             {"bytes", 3},
		"hidden-steady-d1/private_working_set_bytes": {"bytes", 3},
		"hidden-steady-d1/cpu_one_core":              {"percent", 3},
		"visible-mixed-d2/private_bytes":             {"bytes", 3},
		"visible-mixed-d2/private_working_set_bytes": {"bytes", 3},
	}
	seen := map[string]bool{}
	for _, m := range e.Metrics {
		key := m.Scenario + "/" + m.Name
		requirement, ok := required[key]
		if !ok || seen[key] {
			return fmt.Errorf("unknown or duplicate metric: %s", key)
		}
		seen[key] = true
		if m.Unit != requirement.unit || m.SamplingUnit != "independent-run" || len(m.Samples) < requirement.count {
			return fmt.Errorf("invalid unit or insufficient independent runs: %s", key)
		}
		ids, rawPaths := map[string]bool{}, map[string]bool{}
		for _, sample := range m.Samples {
			if sample.RunID == "" || ids[sample.RunID] || rawPaths[sample.Raw.Path] {
				return fmt.Errorf("reused run or raw evidence within %s", key)
			}
			ids[sample.RunID], rawPaths[sample.Raw.Path] = true, true
			if sample.Value == nil || math.IsNaN(*sample.Value) || math.IsInf(*sample.Value, 0) || *sample.Value < 0 {
				return fmt.Errorf("invalid measured value: %s", key)
			}
			if m.Unit != "percent" && *sample.Value == 0 {
				return fmt.Errorf("zero timing/memory is not accepted: %s", key)
			}
			if _, err := checkedArtifact(root, sample.Raw); err != nil {
				return fmt.Errorf("raw %s: %w", key, err)
			}
		}
	}
	for key := range required {
		if !seen[key] {
			return fmt.Errorf("missing metric: %s", key)
		}
	}
	return nil
}

func reservedName(component string) bool {
	name := strings.ToUpper(strings.SplitN(component, ".", 2)[0])
	switch name {
	case "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$":
		return true
	}
	if len(name) == 4 && (strings.HasPrefix(name, "COM") || strings.HasPrefix(name, "LPT")) && name[3] >= '1' && name[3] <= '9' {
		return true
	}
	return false
}
