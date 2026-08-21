import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  initialQuickInsertState,
  quickInsertReducer,
  type QuickInsertReducerAction,
} from "./model/reducer";
import { nextSelection, selectionForKey } from "./model/navigation";
import type {
  ActivePanelChangedEvent,
  FavoriteDraft,
  FavoriteUpdate,
  LibraryChangedEvent,
  PasteSession,
  QuickInsertAction,
  QuickInsertItem,
  QuickInsertState,
  QuickInsertView,
  RuntimeContext,
  StatusKind,
  WindowRole,
} from "./model/types";
import {
  quickInsertClient,
  type QuickInsertClient,
} from "./api/quick-insert-client";

export interface QuickInsertControllerOptions {
  initialSession?: PasteSession | null;
  initialView?: QuickInsertView;
  initialQuery?: string;
  runtimeContext?: RuntimeContext;
  windowRole?: WindowRole;
  onClose: () => void | Promise<void>;
  focusSearch: (select?: boolean) => void;
  client?: QuickInsertClient;
}

export interface QuickInsertController {
  state: QuickInsertState;
  selectedItem: QuickInsertItem | null;
  emptyMessage: string;
  runtimeContext: RuntimeContext;
  setView(view: QuickInsertView): void;
  setQuery(query: string): void;
  setSearchMode(mode: "navigation" | "text-edit"): void;
  enterTextEditMode(): void;
  enterBatchMode(): void;
  cancelBatchMode(): void;
  toggleBatchSelection(id: number): void;
  selectAllBatch(): void;
  select(index: number): void;
  moveSelection(key: string): void;
  confirmSelection(): void;
  execute(item: QuickInsertItem, action?: QuickInsertAction): Promise<void>;
  toggleFavorite(item: QuickInsertItem): Promise<void>;
  remove(item: QuickInsertItem): Promise<void>;
  createFavorite(draft: FavoriteDraft): Promise<QuickInsertItem>;
  updateFavorite(item: QuickInsertItem, update: FavoriteUpdate): Promise<void>;
  reorderFavorites(orderedIds: number[]): Promise<void>;
  togglePin(item: QuickInsertItem): Promise<void>;
  bulkFavorite(ids: number[]): Promise<boolean>;
  bulkPin(ids: number[]): Promise<boolean>;
  bulkDelete(ids: number[]): Promise<boolean>;
  clearUnpinnedHistory(): Promise<void>;
  loadMore(): Promise<void>;
  handleEscape(composing: boolean): void;
  report(status: string, kind?: StatusKind): void;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function restoreAfterInsertFailure(
  focusSearch: (select?: boolean) => void,
) {
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

function eventAffectsView(
  kind: LibraryChangedEvent["kind"],
  view: QuickInsertView,
): boolean {
  return (
    kind === "history_and_favorites" ||
    (kind === "history" && view === "history") ||
    (kind === "favorites" && view === "favorites")
  );
}

export function useQuickInsertController({
  initialSession = null,
  initialView = "history",
  initialQuery = "",
  runtimeContext = "manager",
  windowRole = "main",
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
  const viewRef = useRef(initialView);
  const [loadQuery, setLoadQuery] = useState(initialQuery);

  const report = useCallback((status: string, kind: StatusKind = "info") => {
    dispatch({ type: "status", status, kind });
  }, []);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    const generation = ++generationRef.current;
    dispatch({ type: "load_started", generation });
    try {
      const page = await client.list(state.view, loadQuery, 50, null);
      if (mountedRef.current) {
        dispatch({ type: "load_succeeded", generation, page });
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
  }, [client, loadQuery, state.view]);

  useEffect(() => {
    void load();
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<LibraryChangedEvent>("echo-library-changed", ({ payload }) => {
      if (active && eventAffectsView(payload.kind, state.view)) void load();
    })
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      unlisten?.();
    };
  }, [load, state.view]);

  useEffect(() => {
    if (windowRole !== "main") return;

    let active = true;
    let unlisten: (() => void) | undefined;
    const applyPanel = (view: QuickInsertView) => {
      if (!active || viewRef.current === view) return;
      viewRef.current = view;
      dispatch({ type: "view_changed", view });
      setLoadQuery("");
    };

    void client
      .activePanel()
      .then(applyPanel)
      .catch(() => undefined);
    void listen<ActivePanelChangedEvent>(
      "echo-active-panel-changed",
      ({ payload }) => applyPanel(payload.panel),
    )
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch(() => undefined);

    return () => {
      active = false;
      unlisten?.();
    };
  }, [client, windowRole]);

  useEffect(() => {
    if (state.query === loadQuery) return;
    const timer = window.setTimeout(() => setLoadQuery(state.query), 75);
    return () => window.clearTimeout(timer);
  }, [loadQuery, state.query]);

  const setView = useCallback(
    (view: QuickInsertView) => {
      if (viewRef.current === view) return;
      viewRef.current = view;
      dispatch({ type: "view_changed", view });
      setLoadQuery("");
      if (windowRole === "main") {
        void client.activatePanel(view).catch((error) => {
          report(errorMessage(error), "error");
        });
      }
    },
    [client, report, windowRole],
  );

  const setQuery = useCallback((query: string) => {
    dispatch({ type: "query_changed", query });
  }, []);

  const setSearchMode = useCallback((mode: "navigation" | "text-edit") => {
    dispatch({ type: "search_mode_changed", mode });
  }, []);

  const enterTextEditMode = useCallback(() => {
    setSearchMode("text-edit");
  }, [setSearchMode]);

  const enterBatchMode = useCallback(() => {
    dispatch({ type: "history_mode_changed", mode: "batch" });
  }, []);

  const cancelBatchMode = useCallback(() => {
    dispatch({ type: "history_mode_changed", mode: "browse" });
  }, []);

  const toggleBatchSelection = useCallback((id: number) => {
    dispatch({ type: "batch_selection_toggled", id });
  }, []);

  const selectAllBatch = useCallback(() => {
    dispatch({
      type: "batch_selection_set",
      ids: state.items.map((item) => item.id),
    });
  }, [state.items]);

  const loadMore = useCallback(async () => {
    if (state.loading || state.loadingMore || state.nextCursor === null) return;
    const generation = generationRef.current;
    const cursor = state.nextCursor;
    dispatch({ type: "load_more_started", generation });
    try {
      const page = await client.list(state.view, loadQuery, 50, cursor);
      if (mountedRef.current && generation === generationRef.current) {
        dispatch({ type: "load_more_succeeded", generation, page });
      }
    } catch (error) {
      if (mountedRef.current && generation === generationRef.current) {
        dispatch({
          type: "load_more_failed",
          generation,
          message: errorMessage(error),
        });
      }
    }
  }, [
    client,
    loadQuery,
    state.loading,
    state.loadingMore,
    state.nextCursor,
    state.view,
  ]);

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

  const confirmSelection = useCallback(() => {
    report("Selected", "success");
  }, [report]);

  const execute = useCallback(
    async (item: QuickInsertItem, action: QuickInsertAction = "insert") => {
      if (action === "insert" && runtimeContext === "manager") {
        confirmSelection();
        return;
      }

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
    [client, confirmSelection, focusSearch, onClose, report, runtimeContext],
  );

  const toggleFavorite = useCallback(
    async (item: QuickInsertItem) => {
      if (item.source !== "history") return;
      try {
        await client.moveHistoryToFavorite(item.id);
        report("Added to Favorites", "success");
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, report],
  );

  const remove = useCallback(
    async (item: QuickInsertItem) => {
      try {
        await client.remove(item.source, item.id);
        report("Deleted", "success");
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, report],
  );

  const createFavorite = useCallback(
    async (draft: FavoriteDraft) => {
      try {
        const item = await client.createFavorite(draft);
        report("Favorite created", "success");
        return item;
      } catch (error) {
        report(errorMessage(error), "error");
        throw error;
      }
    },
    [client, report],
  );

  const updateFavorite = useCallback(
    async (item: QuickInsertItem, update: FavoriteUpdate) => {
      try {
        await client.updateFavorite(item.id, update);
        report("Favorite updated", "success");
      } catch (error) {
        report(errorMessage(error), "error");
        throw error;
      }
    },
    [client, report],
  );

  const reorderFavorites = useCallback(
    async (orderedIds: number[]) => {
      try {
        await client.reorderFavorites(orderedIds);
        report("Favorites reordered", "success");
      } catch (error) {
        report(errorMessage(error), "error");
        throw error;
      }
    },
    [client, report],
  );

  const togglePin = useCallback(
    async (item: QuickInsertItem) => {
      if (item.source !== "history") return;
      try {
        if (item.pinned_at === null) {
          await client.pinHistory(item.id);
          report("Pinned", "success");
        } else {
          await client.unpinHistory(item.id);
          report("Unpinned", "success");
        }
      } catch (error) {
        report(errorMessage(error), "error");
      }
    },
    [client, report],
  );

  const bulkFavorite = useCallback(
    async (ids: number[]) => {
      if (ids.length === 0) return false;
      try {
        const moved = await client.moveHistoryManyToFavorites(ids);
        cancelBatchMode();
        report(
          `${moved.length} item${moved.length === 1 ? "" : "s"} added to Favorites`,
          "success",
        );
        return true;
      } catch (error) {
        report(errorMessage(error), "error");
        return false;
      }
    },
    [cancelBatchMode, client, report],
  );

  const bulkPin = useCallback(
    async (ids: number[]) => {
      if (ids.length === 0) return false;
      try {
        await client.pinHistoryMany(ids);
        cancelBatchMode();
        report("History pinned", "success");
        return true;
      } catch (error) {
        report(errorMessage(error), "error");
        return false;
      }
    },
    [cancelBatchMode, client, report],
  );

  const bulkDelete = useCallback(
    async (ids: number[]) => {
      if (ids.length === 0) return false;
      try {
        await client.deleteHistoryMany(ids);
        cancelBatchMode();
        report("History deleted", "success");
        return true;
      } catch (error) {
        report(errorMessage(error), "error");
        return false;
      }
    },
    [cancelBatchMode, client, report],
  );

  const clearUnpinnedHistory = useCallback(async () => {
    try {
      await client.clearUnpinnedHistory();
      cancelBatchMode();
      setQuery("");
      report("History cleared", "success");
    } catch (error) {
      report(errorMessage(error), "error");
      throw error;
    }
  }, [cancelBatchMode, client, report, setQuery]);

  const handleEscape = useCallback(
    (composing: boolean) => {
      if (composing) return;
      if (state.historyMode === "batch") cancelBatchMode();
      void onClose();
    },
    [cancelBatchMode, onClose, state.historyMode],
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
        : "No favorites yet";

  return {
    state,
    selectedItem,
    emptyMessage,
    runtimeContext,
    setView,
    setQuery,
    setSearchMode,
    enterTextEditMode,
    enterBatchMode,
    cancelBatchMode,
    toggleBatchSelection,
    selectAllBatch,
    select,
    moveSelection,
    confirmSelection,
    execute,
    toggleFavorite,
    remove,
    createFavorite,
    updateFavorite,
    reorderFavorites,
    togglePin,
    bulkFavorite,
    bulkPin,
    bulkDelete,
    clearUnpinnedHistory,
    loadMore,
    handleEscape,
    report,
  };
}

export type { QuickInsertReducerAction };
