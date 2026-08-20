import type {
  PasteSession,
  QuickInsertItem,
  QuickInsertView,
  SavedItemUpdate,
} from "../../../shared/ipc/generated";

export type {
  ImagePreview,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertSource,
  QuickInsertView,
  SavedItemUpdate,
} from "../../../shared/ipc/generated";

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
