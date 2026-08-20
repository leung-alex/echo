import { invoke } from "../../../shared/ipc/invoke";

import type {
  ImagePreview,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertSource,
  QuickInsertView,
} from "../model/types";

export interface QuickInsertClient {
  list(
    view: QuickInsertView,
    query: string,
    limit: number,
  ): Promise<QuickInsertItem[]>;
  beginSession(): Promise<PasteSession>;
  execute(
    source: QuickInsertSource,
    id: number,
    action: QuickInsertAction,
  ): Promise<QuickInsertOutcome>;
  setFavorite(
    source: QuickInsertSource,
    id: number,
    pinned: boolean,
  ): Promise<boolean>;
  remove(source: QuickInsertSource, id: number): Promise<boolean>;
  getImage(source: QuickInsertSource, id: number): Promise<string | null>;
}

export const quickInsertClient: QuickInsertClient = {
  list: (view, query, limit) =>
    invoke<QuickInsertItem[]>("quick_insert_list", { view, query, limit }),
  beginSession: () => invoke<PasteSession>("quick_insert_begin_session"),
  execute: (source, id, action) =>
    invoke<QuickInsertOutcome>("quick_insert_execute", { source, id, action }),
  setFavorite: (source, id, pinned) =>
    invoke<boolean>("quick_insert_set_favorite", { source, id, pinned }),
  remove: (source, id) =>
    invoke<boolean>("quick_insert_delete", { source, id }),
  getImage: async (source, id) => {
    const preview = await invoke<ImagePreview | null>(
      "quick_insert_get_image",
      {
        source,
        id,
      },
    );
    return preview
      ? `data:${preview.mime_type};base64,${preview.base64}`
      : null;
  },
};
