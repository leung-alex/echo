import { invoke } from "../../shared/ipc/invoke";

export type { ClipboardSettings } from "../../shared/ipc/generated";
import type { ClipboardSettings } from "../../shared/ipc/generated";

export function getSettings(): Promise<ClipboardSettings> {
  return invoke<ClipboardSettings>("settings_get");
}

export function updateSettings(settings: ClipboardSettings): Promise<void> {
  return invoke<void>("settings_update", { settings });
}

export function clearHistory(): Promise<void> {
  return invoke<number>("quick_insert_clear_unpinned_history").then(
    () => undefined,
  );
}
