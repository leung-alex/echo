package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
)

const generatedTransportPath = "apps/ui/src/shared/ipc/generated.ts"

func generatedTransportBindings() string {
	return `// Generated from apps/desktop/src/transport/mod.rs. Do not edit by hand.
export type QuickInsertView = "history" | "favorites";
export type QuickInsertSource = "history" | "favorite";
export type QuickInsertAction = "copy" | "insert";
export type QuickInsertOutcome = "copied" | "inserted" | "clipboard_staged";
export type ActivationRoute = "history" | "quick_insert" | "settings";

export interface QuickInsertItem {
  id: number;
  source: QuickInsertSource;
  title: string | null;
  preview_text: string | null;
  content_type: string;
  source_app: string | null;
  updated_at: number;
  pinned: boolean;
}

export interface PasteSession {
  hasTarget: boolean;
}

export interface ImagePreview {
  mime_type: string;
  base64: string;
}

export interface ClipboardSettings {
  history_enabled: boolean;
  record_sensitive: boolean;
  store_window_titles: boolean;
  max_entries: number;
  max_total_bytes: number;
  max_item_bytes: number;
}

export interface ActivationPayload {
  route: ActivationRoute;
  query?: string | null;
  request_id: string;
}
`
}

func (a *app) checkGeneratedBindings() error {
	path := filepath.Join(a.root, filepath.FromSlash(generatedTransportPath))
	actual, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("read generated transport bindings: %w", err)
	}
	actual = canonicalizeLineEndings(actual)
	expected := canonicalizeLineEndings([]byte(generatedTransportBindings()))
	if !bytes.Equal(actual, expected) {
		return fmt.Errorf("generated transport bindings are stale; run echo.cmd bindings")
	}
	return nil
}

func canonicalizeLineEndings(input []byte) []byte {
	canonical := bytes.ReplaceAll(input, []byte{'\r', '\n'}, []byte{'\n'})
	return bytes.ReplaceAll(canonical, []byte{'\r'}, []byte{'\n'})
}

func (a *app) writeGeneratedBindings() error {
	path := filepath.Join(a.root, filepath.FromSlash(generatedTransportPath))
	if err := os.WriteFile(path, []byte(generatedTransportBindings()), 0o644); err != nil {
		return fmt.Errorf("write generated transport bindings: %w", err)
	}
	fmt.Fprintf(a.out, "generated %s\n", generatedTransportPath)
	return nil
}
