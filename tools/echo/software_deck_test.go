package main

import (
	"os"
	"path/filepath"
	"regexp"
	"testing"
)

func TestProductionPanelPathsNeverReadBackToCPU(t *testing.T) {
	root, err := findRepositoryRoot(mustGetwd())
	if err != nil {
		t.Fatal(err)
	}
	forbidden := regexp.MustCompile(`\.(take_snapshot|map_async|poll)\s*\(`)
	for _, name := range []string{"app/deck_controller.rs", "app/software_deck.rs"} {
		data, err := os.ReadFile(filepath.Join(root, "apps/desktop/src", name))
		if err != nil {
			t.Fatal(err)
		}
		if forbidden.Match(data) {
			t.Errorf("synchronous/readback API reintroduced into motion path: %s", name)
		}
	}
}
