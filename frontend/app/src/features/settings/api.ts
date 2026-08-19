import { invoke } from "@tauri-apps/api/core";

export interface ClipboardSettings {
  history_enabled: boolean;
  record_sensitive: boolean;
  store_window_titles: boolean;
  max_entries: number;
  max_total_bytes: number;
  max_item_bytes: number;
}

export function getSettings(): Promise<ClipboardSettings> {
  return invoke<ClipboardSettings>("settings_get");
}

export function updateSettings(settings: ClipboardSettings): Promise<void> {
  return invoke<void>("settings_update", { settings });
}

export function clearHistory(): Promise<void> {
  return invoke<void>("history_clear");
}
