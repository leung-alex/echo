package main

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestProductionPanelPathsNeverReadBackToCPU(t *testing.T) {
	root, err := findRepositoryRoot(mustGetwd())
	if err != nil {
		t.Fatal(err)
	}
	forbidden := regexp.MustCompile(`\.(take_snapshot|map_async|poll)\s*\(`)
	for _, name := range []string{"cover_flow/bridge.rs", "cover_flow/offscreen.rs", "cover_flow/compositor.rs", "app/deck_controller.rs", "app/software_deck.rs"} {
		data, err := os.ReadFile(filepath.Join(root, "apps/desktop/src", name))
		if err != nil {
			t.Fatal(err)
		}
		// Hidden reclamation may drain completed retirements without waiting.
		// Keep this exception inside the one lifecycle method; rendering paths,
		// blocking polls and readbacks remain forbidden everywhere below.
		if name == "cover_flow/bridge.rs" {
			source := string(data)
			const start = "    pub fn reclaim_hidden(&self) -> Result<(), String> {"
			if begin := strings.Index(source, start); begin >= 0 {
				if end := strings.Index(source[begin:], "\n    }"); end >= 0 {
					end += begin + len("\n    }")
					method := strings.ReplaceAll(source[begin:end], ".poll(slint::wgpu_29::wgpu::PollType::Poll)", "")
					source = source[:begin] + method + source[end:]
				}
			}
			data = []byte(source)
		}
		if forbidden.Match(data) {
			t.Errorf("synchronous/readback API reintroduced into motion path: %s", name)
		}
	}
}
