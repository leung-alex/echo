export type QuickInsertView = "history" | "favorites" | "snippets";
export type QuickInsertSource = "history" | "favorite" | "snippet";
export type QuickInsertAction = "copy" | "insert";
export type QuickInsertOutcome =
  | "copied"
  | "inserted"
  | "clipboard_staged";

export interface QuickInsertItem {
  id: number;
  source: QuickInsertSource;
  title: string | null;
  preview_text: string | null;
  content_type: string;
  source_app: string | null;
  updated_at: number;
  pinned: boolean;
  group_name: string | null;
}

export interface PasteSession {
  hasTarget: boolean;
}

export interface ImagePreview {
  mime_type: string;
  base64: string;
}

export type StatusKind = "info" | "success" | "error";

export interface QuickInsertState {
  view: QuickInsertView;
  query: string;
  items: QuickInsertItem[];
  selection: number;
  loading: boolean;
  status: string;
  statusKind: StatusKind;
  generation: number;
  session: PasteSession | null;
}
