package main

import (
	"bytes"
	"strings"
	"testing"
)

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
