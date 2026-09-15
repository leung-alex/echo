package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

type designToken struct {
	Runtime bool            `json:"runtime,omitempty"`
	Type    string          `json:"type"`
	Value   json.RawMessage `json:"value"`
	Dark    string          `json:"dark"`
}
type designDocument struct {
	SchemaVersion int                    `json:"schemaVersion"`
	Tokens        map[string]designToken `json:"tokens"`
}

var tokenNamePattern = regexp.MustCompile(`^[a-z][a-z0-9.-]*$`)
var tokenColorPattern = regexp.MustCompile(`^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$`)

// One JSON source generates CSS (reference), Slint and Rust (native runtime).
func renderDesignTokens(data []byte) (map[string][]byte, error) {
	var doc designDocument
	if err := json.Unmarshal(data, &doc); err != nil {
		return nil, fmt.Errorf("read design tokens: %w", err)
	}
	if doc.SchemaVersion != 1 || len(doc.Tokens) == 0 {
		return nil, fmt.Errorf("unsupported or empty token document")
	}
	keys := make([]string, 0, len(doc.Tokens))
	for name := range doc.Tokens {
		keys = append(keys, name)
	}
	sort.Strings(keys)
	var css, dark, slint, rust strings.Builder
	var defaults, setters, schema strings.Builder
	css.WriteString("/* Generated from echo.tokens.json. Reference only; no browser runtime. */\n:root {\n")
	dark.WriteString("[data-theme=\"dark\"] {\n")
	slint.WriteString("// Generated from design/tokens/echo.tokens.json.\nexport global DesignTokens {\n    in-out property <bool> dark: false;\n")
	rust.WriteString("// Generated from design/tokens/echo.tokens.json.\n#![allow(dead_code)]\n")
	for _, name := range keys {
		token := doc.Tokens[name]
		if !tokenNamePattern.MatchString(name) {
			return nil, fmt.Errorf("invalid token name %q", name)
		}
		slug := strings.ReplaceAll(name, ".", "-")
		constant := strings.ToUpper(strings.ReplaceAll(slug, "-", "_"))
		fmt.Fprintf(&schema, "    (%s, %s, %t),\n", strconv.Quote(name), strconv.Quote(token.Type), token.Runtime)
		if token.Runtime {
			if err := renderRuntimeToken(name, slug, token, &css, &dark, &slint, &defaults, &setters); err != nil {
				return nil, err
			}
			continue
		}
		switch token.Type {
		case "color":
			var light string
			if err := json.Unmarshal(token.Value, &light); err != nil {
				return nil, err
			}
			darkValue := token.Dark
			if darkValue == "" {
				darkValue = light
			}
			if !tokenColorPattern.MatchString(light) || !tokenColorPattern.MatchString(darkValue) {
				return nil, fmt.Errorf("invalid color %s", name)
			}
			fmt.Fprintf(&css, "  --echo-%s: %s;\n", slug, light)
			fmt.Fprintf(&dark, "  --echo-%s: %s;\n", slug, darkValue)
			fmt.Fprintf(&slint, "    out property <color> %s: dark ? %s : %s;\n", slug, slintColor(darkValue), slintColor(light))
			fmt.Fprintf(&rust, "pub const %s_LIGHT: &str = %s;\n", constant, strconv.Quote(light))
			fmt.Fprintf(&rust, "pub const %s_DARK: &str = %s;\n", constant, strconv.Quote(darkValue))
		case "string":
			var value string
			if err := json.Unmarshal(token.Value, &value); err != nil {
				return nil, err
			}
			if strings.ContainsAny(value, "\r\n\x00") {
				return nil, fmt.Errorf("multiline token string %s", name)
			}
			fmt.Fprintf(&css, "  --echo-%s: %s;\n", slug, value)
			fmt.Fprintf(&slint, "    out property <string> %s: %s;\n", slug, strconv.Quote(value))
			fmt.Fprintf(&rust, "pub const %s: &str = %s;\n", constant, strconv.Quote(value))
		case "length", "number", "angle", "duration", "integer":
			var value float64
			if err := json.Unmarshal(token.Value, &value); err != nil {
				return nil, err
			}
			number := strconv.FormatFloat(value, 'f', -1, 64)
			unit := ""
			kind := "float"
			switch token.Type {
			case "length":
				unit = "px"
				kind = "length"
			case "angle":
				unit = "deg"
				kind = "angle"
			case "duration":
				unit = "ms"
				kind = "duration"
			case "integer":
				kind = "int"
			}
			if token.Type == "integer" && (value < 0 || value != float64(uint64(value))) {
				return nil, fmt.Errorf("invalid integer token %s", name)
			}
			fmt.Fprintf(&css, "  --echo-%s: %s%s;\n", slug, number, unit)
			fmt.Fprintf(&slint, "    out property <%s> %s: %s%s;\n", kind, slug, number, unit)
			if token.Type == "integer" {
				fmt.Fprintf(&rust, "pub const %s: usize = %s;\n", constant, number)
			} else {
				if !strings.Contains(number, ".") {
					number += ".0"
				}
				fmt.Fprintf(&rust, "pub const %s: f32 = %s;\n", constant, number)
			}
		default:
			return nil, fmt.Errorf("unsupported token type %q for %s", token.Type, name)
		}
	}
	css.WriteString("}\n")
	dark.WriteString("}\n")
	slint.WriteString("}\n")
	return map[string][]byte{
		"apps/desktop/src/style_defaults.rs":          []byte("// Generated from design/tokens/echo.tokens.json.\nuse super::{StyleValue, StyleSnapshot};\n#[cfg(any(debug_assertions, test))]\npub(super) const SCHEMA: &[(&str, &str, bool)] = &[\n" + schema.String() + "];\npub(super) fn defaults() -> StyleSnapshot {\n    StyleSnapshot(std::collections::BTreeMap::from([\n" + defaults.String() + "    ]))\n}\nmacro_rules! apply_style {\n    ($global:expr, $style:expr, $dark:expr) => {{\n        let g = $global;\n        let s = $style;\n        let dark = $dark;\n" + setters.String() + "    }};\n}\npub(crate) use apply_style;\n"),
		"design/tokens/echo.tokens.css":               []byte(css.String() + dark.String()),
		"apps/desktop/ui/echo-tokens.slint":           []byte(slint.String()),
		"crates/echo-presentation/src/echo_tokens.rs": []byte(rust.String()),
	}, nil
}
func slintColor(value string) string {
	if len(value) == 9 {
		alpha, _ := strconv.ParseUint(value[7:9], 16, 8)
		return fmt.Sprintf("%s.with-alpha(%.8f)", value[:7], float64(alpha)/255.0)
	}
	return value
}

func (a *app) generateTokens(check bool) error {
	data, err := os.ReadFile(filepath.Join(a.root, "design", "tokens", "echo.tokens.json"))
	if err != nil {
		return err
	}
	files, err := renderDesignTokens(data)
	if err != nil {
		return err
	}
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		path := filepath.Join(a.root, filepath.FromSlash(name))
		expected := files[name]
		if check {
			actual, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			if !bytes.Equal(canonicalizeLineEndings(actual), expected) {
				return fmt.Errorf("design token drift in %s; run echo.cmd tokens", name)
			}
		} else {
			if actual, err := os.ReadFile(path); err == nil && bytes.Equal(canonicalizeLineEndings(actual), expected) {
				continue
			}
			if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
				return err
			}
			if err := os.WriteFile(path, expected, 0o644); err != nil {
				return err
			}
		}
	}
	return nil
}
