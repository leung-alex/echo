import type {
  HistoryChangedEvent,
  HistoryCursor,
  PasteSession,
  QuickInsertItem,
  QuickInsertView,
  SavedItemUpdate,
} from "../../../shared/ipc/generated";

export type {
  HistoryChangedEvent,
  HistoryCursor,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertPage,
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
  loadingMore: boolean;
  nextCursor: HistoryCursor | null;
  status: string;
  statusKind: StatusKind;
  generation: number;
  session: PasteSession | null;
}
