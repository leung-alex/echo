package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRuntimeValuesDoNotInvalidateCompiledInterfaces(t *testing.T) {
	source := `{"schemaVersion":1,"tokens":{"color.text":{"type":"color","value":"#555b60","runtime":true},"space.2":{"type":"length","value":8,"runtime":true},"budget.page-size":{"type":"integer","value":50}}}`
	before, err := renderDesignTokens([]byte(source))
	if err != nil {
		t.Fatal(err)
	}
	after, err := renderDesignTokens([]byte(strings.ReplaceAll(strings.ReplaceAll(source, "#555b60", "#777777"), `"value":8`, `"value":12`)))
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"apps/desktop/ui/echo-tokens.slint", "crates/echo-presentation/src/echo_tokens.rs"} {
		if !bytes.Equal(before[path], after[path]) {
			t.Fatalf("value edit invalidated %s", path)
		}
	}
	if bytes.Equal(before["apps/desktop/src/style_defaults.rs"], after["apps/desktop/src/style_defaults.rs"]) {
		t.Fatal("embedded defaults did not change")
	}
	if strings.Contains(string(before["apps/desktop/ui/echo-tokens.slint"]), "#555b60") {
		t.Fatal("runtime value leaked into Slint")
	}
}

func TestTokenGenerationPreservesUnchangedFileTimestamp(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "design", "tokens")
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "echo.tokens.json"), []byte(`{"schemaVersion":1,"tokens":{"color.text":{"type":"color","value":"#555b60","runtime":true}}}`), 0644); err != nil {
		t.Fatal(err)
	}
	a := &app{root: root}
	if err := a.generateTokens(false); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(root, "apps", "desktop", "ui", "echo-tokens.slint")
	before, err := os.Stat(file)
	if err != nil {
		t.Fatal(err)
	}
	if err := a.generateTokens(false); err != nil {
		t.Fatal(err)
	}
	after, err := os.Stat(file)
	if err != nil {
		t.Fatal(err)
	}
	if !before.ModTime().Equal(after.ModTime()) {
		t.Fatal("unchanged interface rewritten")
	}
}

func TestDesignTokensGenerateNativeAndReferenceUnits(t *testing.T) {
	data := []byte(`{"schemaVersion":1,"tokens":{"panel.radius":{"type":"length","value":20},"motion.time":{"type":"duration","value":260},"color.shadow":{"type":"color","value":"#00000080","dark":"#ffffff40"}}}`)
	files, err := renderDesignTokens(data)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(files["design/tokens/echo.tokens.css"]), "--echo-panel-radius: 20px") {
		t.Fatal("CSS length unit missing")
	}
	slint := string(files["apps/desktop/ui/echo-tokens.slint"])
	if !strings.Contains(slint, "260ms") || !strings.Contains(slint, "#000000.with-alpha(0.50196078)") {
		t.Fatal(slint)
	}
	if !strings.Contains(string(files["crates/echo-presentation/src/echo_tokens.rs"]), "PANEL_RADIUS: f32 = 20.0") {
		t.Fatal("Rust DIP constant missing")
	}
	again, err := renderDesignTokens(data)
	if err != nil {
		t.Fatal(err)
	}
	for key, value := range files {
		if !bytes.Equal(value, again[key]) {
			t.Fatalf("nondeterministic output %s", key)
		}
	}
}

func TestDesignTokensRejectInvalidSource(t *testing.T) {
	for _, data := range []string{
		`{"schemaVersion":2,"tokens":{"x":{"type":"number","value":1}}}`,
		`{"schemaVersion":1,"tokens":{"../x":{"type":"number","value":1}}}`,
		`{"schemaVersion":1,"tokens":{"x":{"type":"color","value":"red;other"}}}`,
		`{"schemaVersion":1,"tokens":{"x":{"type":"integer","value":1.5}}}`,
		`{"schemaVersion":1,"tokens":{"x":{"type":"function","value":1}}}`,
	} {
		if _, err := renderDesignTokens([]byte(data)); err == nil {
			t.Fatalf("accepted invalid tokens %s", data)
		}
	}
}
