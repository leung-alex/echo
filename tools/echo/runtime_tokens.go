package main

import (
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
)

func renderRuntimeToken(name, slug string, token designToken, css, darkCSS, slint, defaults, setters *strings.Builder) error {
	method := strings.ReplaceAll(slug, "-", "_")
	switch token.Type {
	case "color":
		var light string
		if err := json.Unmarshal(token.Value, &light); err != nil {
			return fmt.Errorf("%s: %w", name, err)
		}
		dark := token.Dark
		if dark == "" {
			dark = light
		}
		if !tokenColorPattern.MatchString(light) || !tokenColorPattern.MatchString(dark) {
			return fmt.Errorf("invalid color %s", name)
		}
		rgba := func(s string) string {
			if len(s) == 7 {
				s += "ff"
			}
			return "0x" + s[1:]
		}
		fmt.Fprintf(css, "  --echo-%s: %s;\n", slug, light)
		fmt.Fprintf(darkCSS, "  --echo-%s: %s;\n", slug, dark)
		fmt.Fprintf(slint, "    in-out property <color> %s;\n", slug)
		fmt.Fprintf(defaults, "        (%s.into(), StyleValue::Color(%s, %s)),\n", strconv.Quote(name), rgba(light), rgba(dark))
		fmt.Fprintf(setters, "        g.set_%s(s.color(%s, dark));\n", method, strconv.Quote(name))
	case "length", "integer":
		var value float64
		if err := json.Unmarshal(token.Value, &value); err != nil {
			return fmt.Errorf("%s: %w", name, err)
		}
		if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || value > 4096 || (strings.HasPrefix(name, "font.") && value < 1) || (token.Type == "integer" && (value != math.Trunc(value) || value > 1000)) {
			return fmt.Errorf("invalid runtime number %s", name)
		}
		if token.Dark != "" {
			return fmt.Errorf("dark override is only valid for runtime colors: %s", name)
		}
		kind, variant, accessor, unit := "length", "Length", "length", "px"
		number := strconv.FormatFloat(value, 'f', -1, 64)
		rustNumber := number
		if token.Type == "integer" {
			kind, variant, accessor, unit = "int", "Integer", "integer", ""
		} else if !strings.Contains(rustNumber, ".") {
			rustNumber += ".0"
		}
		fmt.Fprintf(css, "  --echo-%s: %s%s;\n", slug, number, unit)
		fmt.Fprintf(slint, "    in-out property <%s> %s;\n", kind, slug)
		fmt.Fprintf(defaults, "        (%s.into(), StyleValue::%s(%s)),\n", strconv.Quote(name), variant, rustNumber)
		fmt.Fprintf(setters, "        g.set_%s(s.%s(%s));\n", method, accessor, strconv.Quote(name))
	default:
		return fmt.Errorf("unsupported runtime token type %s for %s", token.Type, name)
	}
	return nil
}
