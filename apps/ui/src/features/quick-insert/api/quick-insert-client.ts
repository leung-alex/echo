import { invoke } from "../../../shared/ipc/invoke";

import type {
  ActivePanelChangedEvent,
  FavoriteDraft,
  FavoriteUpdate,
  LibraryChangedEvent,
  PasteSession,
  QuickInsertAction,
  QuickInsertCursor,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertPage,
  QuickInsertSource,
  QuickInsertView,
} from "../../../shared/ipc/generated";

export interface QuickInsertClient {
  list(
    view: QuickInsertView,
    query: string,
    limit: number,
    cursor: QuickInsertCursor | null,
  ): Promise<QuickInsertPage>;
  beginSession(): Promise<PasteSession>;
  execute(
    source: QuickInsertSource,
    id: number,
    action: QuickInsertAction,
  ): Promise<QuickInsertOutcome>;
  moveHistoryToFavorite(id: number): Promise<QuickInsertItem>;
  moveHistoryManyToFavorites(ids: number[]): Promise<QuickInsertItem[]>;
  createFavorite(draft: FavoriteDraft): Promise<QuickInsertItem>;
  updateFavorite(id: number, update: FavoriteUpdate): Promise<QuickInsertItem>;
  pinHistory(id: number): Promise<boolean>;
  unpinHistory(id: number): Promise<boolean>;
  pinHistoryMany(ids: number[]): Promise<number>;
  deleteHistoryMany(ids: number[]): Promise<number>;
  clearUnpinnedHistory(): Promise<number>;
  reorderFavorites(orderedIds: number[]): Promise<void>;
  deleteFavorite(id: number): Promise<boolean>;
  remove(source: QuickInsertSource, id: number): Promise<boolean>;
  activatePanel(view: QuickInsertView): Promise<void>;
  activePanel(): Promise<QuickInsertView>;
}

export const quickInsertClient: QuickInsertClient = {
  list: (view, query, limit, cursor) =>
    invoke<QuickInsertPage>("quick_insert_list", {
      view,
      query,
      limit,
      cursor,
    }),
  beginSession: () => invoke<PasteSession>("quick_insert_begin_session"),
  execute: (source, id, action) =>
    invoke<QuickInsertOutcome>("quick_insert_execute", { source, id, action }),
  moveHistoryToFavorite: (id) =>
    invoke<QuickInsertItem>("quick_insert_move_history_to_favorite", { id }),
  moveHistoryManyToFavorites: (ids) =>
    invoke<QuickInsertItem[]>("quick_insert_move_history_many_to_favorites", {
      request: { ids },
    }),
  createFavorite: (draft) =>
    invoke<QuickInsertItem>("quick_insert_create_favorite", { draft }),
  updateFavorite: (id, update) =>
    invoke<QuickInsertItem>("quick_insert_update_favorite", { id, update }),
  pinHistory: (id) => invoke<boolean>("quick_insert_pin_history", { id }),
  unpinHistory: (id) => invoke<boolean>("quick_insert_unpin_history", { id }),
  pinHistoryMany: (ids) =>
    invoke<number>("quick_insert_pin_history_many", { request: { ids } }),
  deleteHistoryMany: (ids) =>
    invoke<number>("quick_insert_delete_history_many", { request: { ids } }),
  clearUnpinnedHistory: () =>
    invoke<number>("quick_insert_clear_unpinned_history"),
  reorderFavorites: (orderedIds) =>
    invoke<void>("quick_insert_reorder_favorites", {
      request: { ordered_ids: orderedIds },
    }),
  deleteFavorite: (id) =>
    invoke<boolean>("quick_insert_delete_favorite", { id }),
  remove: (source, id) =>
    source === "history"
      ? invoke<number>("quick_insert_delete_history_many", {
          request: { ids: [id] },
        }).then((deleted) => deleted > 0)
      : invoke<boolean>("quick_insert_delete_favorite", { id }),
  activatePanel: (view) =>
    invoke<void>("quick_insert_activate_panel", { panel: view }),
  activePanel: () => invoke<QuickInsertView>("quick_insert_active_panel"),
};

export type { ActivePanelChangedEvent, LibraryChangedEvent };
