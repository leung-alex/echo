import { invoke } from "../../../shared/ipc/invoke";

import type {
  PasteSession,
  QuickInsertAction,
  HistoryCursor,
  QuickInsertPage,
  QuickInsertOutcome,
  QuickInsertSource,
  QuickInsertView,
  SavedItemUpdate,
} from "../model/types";

export interface QuickInsertClient {
  list(
    view: QuickInsertView,
    query: string,
    limit: number,
    cursor: HistoryCursor | null,
  ): Promise<QuickInsertPage>;
  beginSession(): Promise<PasteSession>;
  execute(
    source: QuickInsertSource,
    id: number,
    action: QuickInsertAction,
  ): Promise<QuickInsertOutcome>;
  setFavorite(
    source: QuickInsertSource,
    id: number,
    saved: boolean,
  ): Promise<boolean>;
  remove(source: QuickInsertSource, id: number): Promise<boolean>;
  updateSavedItem(id: number, update: SavedItemUpdate): Promise<void>;
  deleteSavedItems(ids: number[]): Promise<number>;
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
  setFavorite: (source, id, saved) =>
    invoke<boolean>("quick_insert_set_favorite", { source, id, saved }),
  remove: (source, id) =>
    invoke<boolean>("quick_insert_delete", { source, id }),
  updateSavedItem: (id, update) =>
    invoke<void>("saved_item_update", { id, update }),
  deleteSavedItems: (ids) => invoke<number>("saved_items_delete_many", { ids }),
};
