// Generated from apps/desktop/src/transport/mod.rs. Do not edit by hand.
export type QuickInsertView = "history" | "favorites";
export type QuickInsertSource = "history" | "favorite";
export type QuickInsertAction = "copy" | "insert";
export type QuickInsertOutcome = "copied" | "inserted" | "clipboard_staged";
export type ActivationRoute = "history" | "quick_insert" | "settings";

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
  saved_item_id: number | null;
  is_independent: boolean;
  preview: PreviewAsset | null;
}

export interface PreviewAsset {
  url: string;
  mime_type: string;
  width: number;
  height: number;
  byte_size: number;
  content_hash: string;
}

export interface SavedItemUpdate {
  name: string;
  tags: string[];
  editable_text: string | null;
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
}

export interface ActivationPayload {
  route: ActivationRoute;
  query?: string | null;
  request_id: string;
}
