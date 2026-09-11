package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// All numbers, binaries and pictures in these tests are synthetic. A positive
// integrity test is never evidence that Echo ran or passed Windows acceptance.
func put(t *testing.T, root, name string, data []byte) artifact {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(name))
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	_, hash, err := digest(path)
	if err != nil {
		t.Fatal(err)
	}
	return artifact{name, hash}
}

func evidence(t *testing.T) (string, baselineEvidence) {
	t.Helper()
	root := t.TempDir()
	ref := func(name string) artifact { return put(t, root, name, []byte("synthetic tool unit test: "+name)) }
	e := baselineEvidence{
		Schema: "echo.native.g0.v1", Status: "MEASURED", Variant: "A0", SourceSHA: sourceSHA,
		BuildProfile: "release", WindowCount: 2, Clock: "QPC", ClockFrequency: 10000000,
		ReadinessEndpoint: "external-semantic-ui-ready", ProcessSetVerified: true,
		Environment: environmentEvidence{"windows", "synthetic", true, "synthetic-machine", "synthetic-profile", ref("environment.json")},
		Binary:      ref("runtime/echo-desktop.exe"), Inventory: ref("inventory.json"),
		ProcessOwnership: ref("process-ownership.json"), ObserverOverhead: ref("observer-overhead.json"),
		Fixtures: map[string]fixtureEvidence{"D1": {5000, 200, 0, ref("D1.snapshot")}, "D2": {200, 0, 50, ref("D2.snapshot")}},
		Gates:    map[string]gate{}, Screenshots: map[string]artifact{},
	}
	for _, name := range []string{"self-check", "format", "bindings", "verify", "smoke", "storage-perf", "acceptance-clipboard", "acceptance-quick-insert", "visual-parity"} {
		e.Gates[name] = gate{"PASS", ref("gates/" + name + ".log")}
	}
	for index, name := range []string{"history-light", "history-dark", "favorites", "hover", "settings", "mixed-images"} {
		picture := image.NewNRGBA(image.Rect(0, 0, 2, 2))
		picture.SetNRGBA(0, 0, color.NRGBA{R: uint8(index + 1), A: 255})
		var buffer bytes.Buffer
		if err := png.Encode(&buffer, picture); err != nil {
			t.Fatal(err)
		}
		e.Screenshots[name] = put(t, root, "screenshots/"+name+".png", buffer.Bytes())
	}
	for _, item := range []struct {
		scenario, name, unit string
		count                int
	}{
		{"process-start-d1", "ui_ready", "ms", 30},
		{"hot-show-d1", "ui_ready", "ms", 50},
		{"search-d1", "end_to_end", "ms", 50},
		{"hidden-steady-d1", "private_bytes", "bytes", 3},
		{"hidden-steady-d1", "private_working_set_bytes", "bytes", 3},
		{"hidden-steady-d1", "cpu_one_core", "percent", 3},
		{"visible-mixed-d2", "private_bytes", "bytes", 3},
		{"visible-mixed-d2", "private_working_set_bytes", "bytes", 3},
	} {
		m := metric{Scenario: item.scenario, Name: item.name, Unit: item.unit, SamplingUnit: "independent-run"}
		for index := 0; index < item.count; index++ {
			value := float64(index + 1)
			id := fmt.Sprintf("%s-%s-%03d", item.scenario, item.name, index)
			m.Samples = append(m.Samples, measurement{id, &value, ref("raw/" + id + ".json")})
		}
		e.Metrics = append(e.Metrics, m)
	}
	return root, e
}

func saveEvidence(t *testing.T, root string, e baselineEvidence) {
	t.Helper()
	data, err := json.Marshal(e)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "g0.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestEvidenceIntegrityAcceptsCompleteSyntheticBundle(t *testing.T) {
	root, e := evidence(t)
	saveEvidence(t, root, e)
	if err := validateG0(root); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := command([]string{"check-g0", "--bundle", root}, &out); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "independent semantic/visual review still required") {
		t.Fatal("must not certify native acceptance")
	}
}

func TestEvidenceRejectsMissingOrUntrustworthyRuns(t *testing.T) {
	tests := []struct {
		name   string
		change func(*baselineEvidence)
	}{
		{"not-run", func(e *baselineEvidence) { e.Status = "NOT_RUN" }},
		{"candidate-is-not-baseline", func(e *baselineEvidence) { e.Variant = "C" }},
		{"wrong-source", func(e *baselineEvidence) { e.SourceSHA = strings.Repeat("1", 40) }},
		{"debug-build", func(e *baselineEvidence) { e.BuildProfile = "debug" }},
		{"single-window", func(e *baselineEvidence) { e.WindowCount = 1 }},
		{"wall-clock", func(e *baselineEvidence) { e.Clock = "UTC" }},
		{"missing-qpc-frequency", func(e *baselineEvidence) { e.ClockFrequency = 0 }},
		{"hwnd-only-ready", func(e *baselineEvidence) { e.ReadinessEndpoint = "WaitForInputIdle" }},
		{"linux-is-not-windows", func(e *baselineEvidence) { e.Environment.OS = "linux" }},
		{"no-interactive-session", func(e *baselineEvidence) { e.Environment.InteractiveSession = false }},
		{"no-environment-profile", func(e *baselineEvidence) { e.Environment.ComparisonProfileID = "" }},
		{"unverified-processes", func(e *baselineEvidence) { e.ProcessSetVerified = false }},
		{"no-observer-overhead", func(e *baselineEvidence) { e.ObserverOverhead = artifact{} }},
		{"missing-gate", func(e *baselineEvidence) { delete(e.Gates, "verify") }},
		{"failed-native-gate", func(e *baselineEvidence) {
			g := e.Gates["acceptance-clipboard"]
			g.Status = "FAIL"
			e.Gates["acceptance-clipboard"] = g
		}},
		{"not-run-visual-gate", func(e *baselineEvidence) {
			g := e.Gates["visual-parity"]
			g.Status = "NOT_RUN"
			e.Gates["visual-parity"] = g
		}},
		{"missing-d1", func(e *baselineEvidence) { delete(e.Fixtures, "D1") }},
		{"wrong-d1-count", func(e *baselineEvidence) { d := e.Fixtures["D1"]; d.History = 10; e.Fixtures["D1"] = d }},
		{"wrong-image-count", func(e *baselineEvidence) { d := e.Fixtures["D2"]; d.Images = 0; e.Fixtures["D2"] = d }},
		{"missing-screenshot", func(e *baselineEvidence) { delete(e.Screenshots, "hover") }},
		{"same-screenshot-for-two-states", func(e *baselineEvidence) { e.Screenshots["hover"] = e.Screenshots["settings"] }},
		{"missing-metric", func(e *baselineEvidence) { e.Metrics = e.Metrics[1:] }},
		{"duplicate-metric", func(e *baselineEvidence) { e.Metrics = append(e.Metrics, e.Metrics[0]) }},
		{"unknown-metric", func(e *baselineEvidence) { e.Metrics[0].Name = "time-to-hwnd" }},
		{"insufficient-startup-runs", func(e *baselineEvidence) { e.Metrics[0].Samples = e.Metrics[0].Samples[:29] }},
		{"continuous-samples-are-not-independent", func(e *baselineEvidence) { e.Metrics[3].SamplingUnit = "continuous-one-second-sample" }},
		{"wrong-unit", func(e *baselineEvidence) { e.Metrics[0].Unit = "seconds" }},
		{"duplicate-run-id", func(e *baselineEvidence) { e.Metrics[0].Samples[1].RunID = e.Metrics[0].Samples[0].RunID }},
		{"same-raw-file-for-two-runs", func(e *baselineEvidence) { e.Metrics[0].Samples[1].Raw = e.Metrics[0].Samples[0].Raw }},
		{"null-value", func(e *baselineEvidence) { e.Metrics[0].Samples[0].Value = nil }},
		{"negative-value", func(e *baselineEvidence) { v := -1.0; e.Metrics[0].Samples[0].Value = &v }},
		{"zero-time", func(e *baselineEvidence) { v := 0.0; e.Metrics[0].Samples[0].Value = &v }},
		{"missing-raw-file", func(e *baselineEvidence) { e.Metrics[0].Samples[0].Raw.Path = "missing.csv" }},
		{"hash-mismatch", func(e *baselineEvidence) { e.Binary.SHA256 = strings.Repeat("0", 64) }},
		{"upper-case-hash", func(e *baselineEvidence) { e.Binary.SHA256 = strings.ToUpper(e.Binary.SHA256) }},
		{"path-traversal", func(e *baselineEvidence) { e.Binary.Path = "../secret.exe" }},
		{"absolute-path", func(e *baselineEvidence) { e.Binary.Path = "/secret.exe" }},
		{"alternate-data-stream", func(e *baselineEvidence) { e.Binary.Path = "runtime/app.exe:stream" }},
		{"windows-device", func(e *baselineEvidence) { e.Binary.Path = "runtime/CON.exe" }},
		{"windows-path-alias", func(e *baselineEvidence) { e.Binary.Path = "runtime/file. " }},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			root, e := evidence(t)
			test.change(&e)
			saveEvidence(t, root, e)
			if err := validateG0(root); err == nil {
				t.Fatal("invalid evidence must not pass")
			}
		})
	}
}

func TestChangedAndEmptyEvidenceRejected(t *testing.T) {
	for _, data := range [][]byte{nil, []byte("changed")} {
		root, e := evidence(t)
		saveEvidence(t, root, e)
		if err := os.WriteFile(filepath.Join(root, e.Binary.Path), data, 0o600); err != nil {
			t.Fatal(err)
		}
		if err := validateG0(root); err == nil {
			t.Fatal("changed/empty evidence passed")
		}
	}
}

func TestScreenshotMustHaveValidPNGMetadata(t *testing.T) {
	root, e := evidence(t)
	e.Screenshots["hover"] = put(t, root, "bad.png", []byte{137, 80, 78, 71, 13, 10, 26, 10})
	saveEvidence(t, root, e)
	if err := validateG0(root); err == nil {
		t.Fatal("PNG signature alone is not a picture")
	}
}

func TestStrictJSON(t *testing.T) {
	for _, data := range []string{
		`{"status":"NOT_RUN","status":"MEASURED"}`,
		`{"nested":{"x":1,"x":2}}`,
		`[{"x":1,"x":2}]`,
		`{} {}`,
		`{"samples":[NaN]}`,
	} {
		if err := rejectDuplicateKeys([]byte(data)); err == nil {
			t.Fatalf("accepted %s", data)
		}
	}
	root, e := evidence(t)
	data, _ := json.Marshal(e)
	data = append([]byte(`{"unknown":1,`), data[1:]...)
	put(t, root, "g0.json", data)
	if err := validateG0(root); err == nil {
		t.Fatal("unknown field accepted")
	}
	put(t, root, "g0.json", bytes.Repeat([]byte(" "), maxJSONBytes+1))
	if err := validateG0(root); err == nil {
		t.Fatal("oversized JSON accepted")
	}
}

func TestNewEvidenceCannotOverwriteFile(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "report.json")
	if err := writeNewJSON(path, map[string]string{"status": "NOT_RUN"}); err != nil {
		t.Fatal(err)
	}
	before, _ := os.ReadFile(path)
	if err := writeNewJSON(path, map[string]string{"status": "PASS"}); err == nil {
		t.Fatal("overwritten")
	}
	after, _ := os.ReadFile(path)
	if !bytes.Equal(before, after) {
		t.Fatal("old evidence modified")
	}
}

func TestOutputMustBeOutsideInputs(t *testing.T) {
	root := t.TempDir()
	for _, path := range []string{root, filepath.Join(root, "report.json"), filepath.Join(root, "nested", "report.json")} {
		if err := outside(root, path); err == nil {
			t.Fatalf("accepted output %s", path)
		}
	}
	if err := outside(root, root+"-evidence/report.json"); err != nil {
		t.Fatal(err)
	}
}

func TestLinksAreRejected(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(t.TempDir(), "real")
	if err := os.WriteFile(target, []byte("private"), 0o600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symlink privilege unavailable: %v", err)
	}
	if _, _, err := digest(link); err == nil {
		t.Fatal("file symlink accepted")
	}
	if _, err := inventory(root, ""); err == nil {
		t.Fatal("runtime symlink accepted")
	}
	parent := filepath.Join(root, "alias")
	if err := os.Symlink(filepath.Dir(target), parent); err != nil {
		t.Fatal(err)
	}
	if err := writeNewJSON(filepath.Join(parent, "new.json"), map[string]int{"a": 1}); err == nil {
		t.Fatal("symlink parent accepted")
	}
}

func TestInventoryCountsAllRuntimeFilesSeparatelyFromInstaller(t *testing.T) {
	parent := t.TempDir()
	root := filepath.Join(parent, "runtime")
	put(t, root, "app.exe", []byte("1234"))
	put(t, root, "assets/image.png", []byte("12345"))
	installer := put(t, parent, "setup.exe", []byte("123456"))
	result, err := inventory(root, filepath.Join(parent, installer.Path))
	if err != nil {
		t.Fatal(err)
	}
	if result.RuntimeBytes != 9 || len(result.Files) != 2 || result.Installer.Bytes != 6 {
		t.Fatalf("wrong inventory: %+v", result)
	}
	if result.Files[0].Path != "app.exe" || !validHash(result.Files[0].SHA256, 64) {
		t.Fatal("inventory not deterministic/hash-bound")
	}
}

func TestInventoryEmptyAndNonDirectoryRejected(t *testing.T) {
	root := t.TempDir()
	if _, err := inventory(root, ""); err == nil {
		t.Fatal("empty runtime accepted")
	}
	file := put(t, root, "file", []byte("x"))
	if _, err := inventory(filepath.Join(root, file.Path), ""); err == nil {
		t.Fatal("file as runtime accepted")
	}
}

func TestCLIInvalidArguments(t *testing.T) {
	for _, args := range [][]string{
		{"unknown"}, {"preflight"}, {"inventory"}, {"check-g0"},
		{"inventory", "--runtime", "x", "--out", "y", "extra"},
		{"preflight", "--unknown", "x"},
	} {
		if err := command(args, &bytes.Buffer{}); err == nil {
			t.Fatalf("accepted %v", args)
		}
	}
	if err := command([]string{"help"}, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
}

func TestPreparationWhitelistIsNarrow(t *testing.T) {
	for _, path := range []string{"apps/desktop/src/lib.rs", "Cargo.lock", "tools/echo/main.go", "tools/echo/perf-native-escape.go", "docs/migration/other.md", ".github/workflows/other.yml", ".github/workflows/echo-native-perf.yml", " tools/echo/perf-native/main.go"} {
		if preparationOnly(path) {
			t.Fatalf("allowed unapproved change %s", path)
		}
	}
	for _, path := range []string{"tools/echo/perf-native/main.go", "tools/perf-native/README.md", "docs/migration/slint/status.md"} {
		if !preparationOnly(path) {
			t.Fatalf("rejected M0 preparation %s", path)
		}
	}
}

func TestNonWindowsPreflightNeverReleasesG0(t *testing.T) {
	report := preflight(t.TempDir(), "linux", "")
	if report.G0 != "NOT_RUN" || report.Status != "BLOCKED" {
		t.Fatalf("false acceptance: %+v", report)
	}
	if !strings.Contains(strings.Join(report.Blockers, " "), "real Windows") {
		t.Fatal("missing Windows blocker")
	}
}
