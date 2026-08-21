import type {
  PasteSession,
  QuickInsertCursor,
  QuickInsertItem,
  QuickInsertView,
} from "../../../shared/ipc/generated";

export type {
  ActivePanelChangedEvent,
  FavoriteDraft,
  FavoriteReorderRequest,
  FavoriteUpdate,
  HistoryChangedEvent,
  HistoryIds,
  LibraryChangedEvent,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertPage,
  QuickInsertSource,
  QuickInsertView,
} from "../../../shared/ipc/generated";
export type { HistoryMode, SearchMode } from "./interaction";

export type RuntimeContext = "manager" | "quick-insert";
export type WindowRole = "main" | "favorites";

export type StatusKind = "info" | "success" | "error";

export interface QuickInsertState {
  view: QuickInsertView;
  query: string;
  items: QuickInsertItem[];
  selection: number;
  loading: boolean;
  loadingMore: boolean;
  nextCursor: QuickInsertCursor | null;
  status: string;
  statusKind: StatusKind;
  generation: number;
  session: PasteSession | null;
  searchMode: import("./interaction").SearchMode;
  historyMode: import("./interaction").HistoryMode;
  batchSelectedIds: number[];
}
