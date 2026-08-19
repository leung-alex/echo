import { invoke } from "@tauri-apps/api/core";

import type {
  ImagePreview,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertOutcome,
  QuickInsertSource,
  QuickInsertView,
} from "../model/types";

export interface EchoSnippet {
  id: number;
  name: string;
  content: string;
  group_name: string | null;
  created_at: number;
  updated_at: number;
}

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
  listSnippets(query: string): Promise<EchoSnippet[]>;
  saveSnippet(
    id: number | null,
    name: string,
    content: string,
    groupName: string | null,
  ): Promise<number>;
  deleteSnippet(id: number): Promise<boolean>;
}

function parseOutcome(raw: string): QuickInsertOutcome {
  const outcome = JSON.parse(raw) as unknown;
  if (
    outcome !== "copied" &&
    outcome !== "inserted" &&
    outcome !== "clipboard_staged"
  ) {
    throw new Error("Echo returned an unknown Quick Insert outcome");
  }
  return outcome;
}

export const quickInsertClient: QuickInsertClient = {
  list: (view, query, limit) =>
    invoke<QuickInsertItem[]>("quick_insert_list", { view, query, limit }),
  beginSession: async () => ({
    hasTarget: await invoke<boolean>("quick_insert_begin_session"),
  }),
  execute: async (source, id, action) =>
    parseOutcome(
      await invoke<string>("quick_insert_execute", { source, id, action }),
    ),
  setFavorite: (source, id, pinned) =>
    invoke<boolean>("quick_insert_set_favorite", { source, id, pinned }),
  remove: (source, id) => invoke<boolean>("quick_insert_delete", { source, id }),
  getImage: async (source, id) => {
    const preview = await invoke<ImagePreview | null>("quick_insert_get_image", {
      source,
      id,
    });
    return preview
      ? `data:${preview.mime_type};base64,${preview.base64}`
      : null;
  },
  listSnippets: (query) => invoke<EchoSnippet[]>("snippets_list", { query }),
  saveSnippet: (id, name, content, groupName) =>
    invoke<number>("snippet_save", {
      id,
      name,
      content,
      group_name: groupName,
    }),
  deleteSnippet: (id) => invoke<boolean>("snippet_delete", { id }),
};
