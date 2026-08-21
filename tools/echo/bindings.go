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
export type ThemeMode = "system" | "light" | "dark";

export type QuickInsertCursor =
  | {
      kind: "history";
      pinned_at: number | null;
      updated_at: number;
      id: number;
    }
  | {
      kind: "history_search";
      pinned_at: number | null;
      relevance: number;
      updated_at: number;
      id: number;
    }
  | { kind: "favorites"; favorite_order: number; id: number }
  | {
      kind: "favorites_search";
      relevance: number;
      favorite_order: number;
      id: number;
    };

export interface QuickInsertItem {
  id: number;
  source: QuickInsertSource;
  name: string | null;
  preview_text: string | null;
  content_type: string;
  editable_text: string | null;
  tags: string[];
  source_app: string | null;
  updated_at: number;
  pinned_at: number | null;
  icon_key: string | null;
  favorite_order: number | null;
  preview: PreviewAsset | null;
}

export interface QuickInsertPage {
  items: QuickInsertItem[];
  next_cursor: QuickInsertCursor | null;
}

export interface HistoryChangedEvent {
  version: number;
}

export type LibraryChangeKind =
  | "history"
  | "favorites"
  | "history_and_favorites";

export interface LibraryChangedEvent {
  version: number;
  kind: LibraryChangeKind;
}

export interface ActivePanelChangedEvent {
  panel: QuickInsertView;
}

export interface PreviewAsset {
  url: string;
  mime_type: string;
  width: number;
  height: number;
  byte_size: number;
  content_hash: string;
}

export interface FavoriteDraft {
  content: string;
  name: string | null;
  icon_key: string | null;
  tags: string[];
}

export interface FavoriteUpdate {
  name: string | null;
  icon_key: string | null;
  tags: string[];
  editable_text: string | null;
}

export interface HistoryIds {
  ids: number[];
}

export interface FavoriteReorderRequest {
  ordered_ids: number[];
}

export interface PasteSession {
  hasTarget: boolean;
}

export interface ClipboardSettings {
  history_enabled: boolean;
  record_sensitive: boolean;
  store_window_titles: boolean;
  max_entries: number;
  max_total_bytes: number;
  max_item_bytes: number;
  theme: ThemeMode;
}

export interface ThemeChangedEvent {
  mode: ThemeMode;
  nativeMica: boolean;
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
