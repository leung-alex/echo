import type {
  QuickInsertPage,
  QuickInsertState,
  QuickInsertView,
  StatusKind,
} from "./types";

export type QuickInsertReducerAction =
  | { type: "view_changed"; view: QuickInsertView }
  | { type: "query_changed"; query: string }
  | { type: "load_started"; generation: number }
  | {
      type: "load_succeeded";
      generation: number;
      page: QuickInsertPage;
    }
  | { type: "load_more_started"; generation: number }
  | {
      type: "load_more_succeeded";
      generation: number;
      page: QuickInsertPage;
    }
  | { type: "load_more_failed"; generation: number; message: string }
  | { type: "load_failed"; generation: number; message: string }
  | { type: "selection_changed"; selection: number }
  | { type: "status"; status: string; kind?: StatusKind }
  | { type: "session_changed"; session: QuickInsertState["session"] };

export function initialQuickInsertState(
  view: QuickInsertView = "history",
  query = "",
  session: QuickInsertState["session"] = null,
): QuickInsertState {
  return {
    view,
    query,
    items: [],
    selection: -1,
    loading: true,
    loadingMore: false,
    nextCursor: null,
    status: "Loading",
    statusKind: "info",
    generation: 0,
    session,
  };
}

export function quickInsertReducer(
  state: QuickInsertState,
  action: QuickInsertReducerAction,
): QuickInsertState {
  switch (action.type) {
    case "view_changed":
      return {
        ...state,
        view: action.view,
        query: "",
        items: [],
        selection: -1,
        loading: true,
        loadingMore: false,
        nextCursor: null,
        status: "Loading",
        statusKind: "info",
      };
    case "query_changed":
      return {
        ...state,
        query: action.query,
        selection: -1,
        loading: true,
        loadingMore: false,
        nextCursor: null,
        status: "Loading",
        statusKind: "info",
      };
    case "load_started":
      return {
        ...state,
        loading: true,
        loadingMore: false,
        generation: action.generation,
      };
    case "load_succeeded": {
      if (action.generation !== state.generation) return state;
      const selection =
        action.page.items.length === 0
          ? -1
          : Math.min(
              Math.max(state.selection, 0),
              action.page.items.length - 1,
            );
      const preserveSuccess = state.statusKind === "success";
      return {
        ...state,
        items: action.page.items,
        selection,
        loading: false,
        loadingMore: false,
        nextCursor: action.page.next_cursor,
        status: preserveSuccess
          ? state.status
          : `${action.page.items.length} item${action.page.items.length === 1 ? "" : "s"}`,
        statusKind: preserveSuccess ? "success" : "info",
      };
    }
    case "load_more_started":
      if (action.generation !== state.generation) return state;
      return { ...state, loadingMore: true };
    case "load_more_succeeded": {
      if (action.generation !== state.generation) return state;
      const existing = new Set(
        state.items.map((item) => `${item.source}:${item.id}`),
      );
      const appended = action.page.items.filter(
        (item) => !existing.has(`${item.source}:${item.id}`),
      );
      const total = state.items.length + appended.length;
      return {
        ...state,
        items: [...state.items, ...appended],
        loadingMore: false,
        nextCursor: action.page.next_cursor,
        status: `${total} item${total === 1 ? "" : "s"}`,
        statusKind: "info",
      };
    }
    case "load_more_failed":
      if (action.generation !== state.generation) return state;
      return {
        ...state,
        loadingMore: false,
        status: action.message,
        statusKind: "error",
      };
    case "load_failed":
      if (action.generation !== state.generation) return state;
      return {
        ...state,
        items: [],
        selection: -1,
        loading: false,
        loadingMore: false,
        nextCursor: null,
        status: action.message,
        statusKind: "error",
      };
    case "selection_changed":
      return {
        ...state,
        selection: action.selection,
      };
    case "status":
      return {
        ...state,
        status: action.status,
        statusKind: action.kind ?? "info",
      };
    case "session_changed":
      return { ...state, session: action.session };
  }
}
