// Generated from apps/desktop/src/transport/mod.rs. Do not edit by hand.
export type QuickInsertView = "history" | "favorites";
export type QuickInsertSource = "history" | "favorite";
export type QuickInsertAction = "copy" | "insert";
export type QuickInsertOutcome = "copied" | "inserted" | "clipboard_staged";

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
  route: "history" | "quick_insert" | "settings";
  query?: string | null;
  request_id?: string;
}
