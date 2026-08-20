import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  initialQuickInsertState,
  quickInsertReducer,
  type QuickInsertReducerAction,
} from "./model/reducer";
import { nextSelection, selectionForKey } from "./model/navigation";
import type {
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertState,
  QuickInsertView,
  SavedItemUpdate,
  StatusKind,
} from "./model/types";
import {
  quickInsertClient,
  type QuickInsertClient,
} from "./api/quick-insert-client";

export interface QuickInsertControllerOptions {
  initialSession?: PasteSession | null;
  initialView?: QuickInsertView;
  initialQuery?: string;
  onClose: () => void | Promise<void>;
  focusSearch: () => void;
  client?: QuickInsertClient;
}

export interface QuickInsertController {
  state: QuickInsertState;
  selectedItem: QuickInsertItem | null;
  emptyMessage: string;
  setView(view: QuickInsertView): void;
  setQuery(query: string): void;
  select(index: number): void;
  moveSelection(key: string): void;
  execute(item: QuickInsertItem, action?: QuickInsertAction): Promise<void>;
  toggleFavorite(item: QuickInsertItem): Promise<void>;
  remove(item: QuickInsertItem): Promise<void>;
  updateSavedItem(
    item: QuickInsertItem,
    update: SavedItemUpdate,
  ): Promise<void>;
  deleteSavedItems(ids: number[]): Promise<boolean>;
  handleEscape(composing: boolean): void;
  report(status: string, kind?: StatusKind): void;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function restoreAfterInsertFailure(focusSearch: () => void) {
  try {
    const window = getCurrentWindow();
    await window.show();
    await window.setFocusable(true);
    await window.setFocus();
  } catch {
    // Browser-owned tests and an already-visible window can skip restoration.
  }
  focusSearch();
}

export function useQuickInsertController({
  initialSession = null,
  initialView = "history",
  initialQuery = "",
  onClose,
  focusSearch,
  client = quickInsertClient,
}: QuickInsertControllerOptions): QuickInsertController {
  const [state, dispatch] = useReducer(
    quickInsertReducer,
    initialQuickInsertState(initialView, initialQuery, initialSession),
  );
  const generationRef = useRef(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    const generation = ++generationRef.current;
    dispatch({ type: "load_started", generation });
    try {
      const items = await client.list(state.view, state.query, 200);
      if (mountedRef.current) {
        dispatch({ type: "load_succeeded", generation, items });
      }
    } catch (error) {
      if (mountedRef.current) {
        dispatch({
          type: "load_failed",
          generation,
          message: errorMessage(error),
        });
      }
    }
  }, [client, state.query, state.view]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 900);
    return () => window.clearInterval(timer);
  }, [load]);

  const setView = useCallback((view: QuickInsertView) => {
    dispatch({ type: "view_changed", view });
  }, []);

  const setQuery = useCallback((query: string) => {
    dispatch({ type: "query_changed", query });
  }, []);

  const select = useCallback((index: number) => {
    dispatch({ type: "selection_changed", selection: index });
  }, []);

  const moveSelection = useCallback(
    (key: string) => {
      const direction = key === "ArrowUp" || key === "k" ? -1 : 1;
      const numeric = selectionForKey(key, state.items.length);
      const selection =
        numeric ??
        nextSelection(state.selection, state.items.length, direction);
      if (selection >= 0) select(selection);
    },
    [select, state.items.length, state.selection],
  );

  const report = useCallback((status: string, kind: StatusKind = "info") => {
    dispatch({ type: "status", status, kind });
  }, []);

  const execute = useCallback(
    async (item: QuickInsertItem, action: QuickInsertAction = "insert") => {
      report(action === "insert" ? "Inserting..." : "Copying...");
      let hiddenForInsert = false;
      try {
        if (action === "insert") {
          try {
            await getCurrentWindow().hide();
            hiddenForInsert = true;
          } catch {
            // Keep the command recoverable if the window cannot be hidden.
          }
        }
        const outcome = await client.execute(item.source, item.id, action);
        const message =
          outcome === "inserted"
            ? "Inserted"
            : outcome === "copied"
              ? "Copied"
              : "Clipboard staged";
        report(message, "success");
        if (outcome === "inserted") {
          if (!hiddenForInsert) await onClose();
        } else if (hiddenForInsert) {
          await restoreAfterInsertFailure(focusSearch);
        }
      } catch (error) {
        if (hiddenForInsert) await restoreAfterInsertFailure(focusSearch);
        report(errorMessage(error), "error");
        if (!hiddenForInsert) focusSearch();
      }
    },
    [client, focusSearch, onClose, report],
  );

  const toggleFavorite = useCallback(
    async (item: QuickInsertItem) => {
      try {
        const saved = item.source === "favorite" || item.saved_item_id !== null;
        await client.setFavorite(item.source, item.id, !saved);
        await load();
        report(
          saved ? "Removed from Favorites" : "Added to Favorites",
          "success",
        );
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, load, report],
  );

  const remove = useCallback(
    async (item: QuickInsertItem) => {
      try {
        await client.remove(item.source, item.id);
        await load();
        report("Deleted", "success");
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, load, report],
  );

  const updateSavedItem = useCallback(
    async (item: QuickInsertItem, update: SavedItemUpdate) => {
      try {
        await client.updateSavedItem(item.id, update);
        await load();
        report("Saved item updated", "success");
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, load, report],
  );

  const deleteSavedItems = useCallback(
    async (ids: number[]): Promise<boolean> => {
      if (ids.length === 0) return false;
      try {
        await client.deleteSavedItems(ids);
        await load();
        report("Deleted", "success");
        return true;
      } catch (error) {
        report(errorMessage(error), "error");
        return false;
      }
    },
    [client, load, report],
  );

  const handleEscape = useCallback(
    (composing: boolean) => {
      if (composing) return;
      if (state.query) setQuery("");
      else void onClose();
    },
    [onClose, setQuery, state.query],
  );

  const selectedItem = useMemo(
    () => state.items[state.selection] ?? null,
    [state.items, state.selection],
  );
  const emptyMessage = state.loading
    ? "Loading..."
    : state.statusKind === "error"
      ? "Unable to load results"
      : state.view === "history"
        ? "No clipboard history"
        : state.view === "favorites"
          ? "No favorites yet"
          : "No clipboard history";

  return {
    state,
    selectedItem,
    emptyMessage,
    setView,
    setQuery,
    select,
    moveSelection,
    execute,
    toggleFavorite,
    remove,
    updateSavedItem,
    deleteSavedItems,
    handleEscape,
    report,
  };
}

export type { QuickInsertReducerAction };
