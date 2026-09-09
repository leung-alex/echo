package main

import (
	"strings"
	"testing"
)

func TestInlineGateRejectsUnknownScopeBeforeAnySideEffect(t *testing.T) {
	a := &app{}
	if err := a.runInlineGate("other"); err == nil || !strings.Contains(err.Error(), "unsupported") {
		t.Fatalf("unknown native scope was accepted: %v", err)
	}
}

func TestInlineGateRequiresExplicitDesktopAuthorization(t *testing.T) {
	t.Setenv("ECHO_WINDOWS_ACCEPTANCE", "")
	a := &app{}
	for _, scope := range []string{"clipboard", "quick-insert"} {
		if err := a.runNativeGate(scope); err == nil || !strings.Contains(err.Error(), "separately authorized") {
			t.Fatalf("%s did not reject unauthorized execution: %v", scope, err)
		}
	}
}
