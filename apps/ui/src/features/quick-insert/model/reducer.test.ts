import { describe, expect, it } from "vitest";

import { initialQuickInsertState, quickInsertReducer } from "./reducer";

const item = {
  id: 1,
  source: "history" as const,
  name: null,
  preview_text: "Hello",
  content_type: "text",
  editable_text: null,
  tags: [],
  source_app: "Fixture",
  updated_at: 1,
  pinned_at: null,
  icon_key: null,
  favorite_order: null,
  preview: null,
};

describe("quick insert reducer", () => {
  it("keeps only the newest async load", () => {
    let state = initialQuickInsertState();
    state = quickInsertReducer(state, { type: "load_started", generation: 1 });
    state = quickInsertReducer(state, { type: "load_started", generation: 2 });
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 1,
      page: { items: [item], next_cursor: null },
    });
    expect(state.items).toEqual([]);
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 2,
      page: { items: [item], next_cursor: null },
    });
    expect(state.items).toEqual([item]);
    expect(state.selection).toBe(0);
  });

  it("resets query and selection when switching library views", () => {
    let state = initialQuickInsertState("history", "hello");
    state = quickInsertReducer(state, { type: "load_started", generation: 1 });
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 1,
      page: { items: [item], next_cursor: null },
    });
    state = quickInsertReducer(state, {
      type: "view_changed",
      view: "favorites",
    });
    expect(state.view).toBe("favorites");
    expect(state.query).toBe("");
    expect(state.selection).toBe(-1);
    expect(state.loading).toBe(true);
  });

  it("does not erase a successful action status during background refresh", () => {
    let state = initialQuickInsertState();
    state = quickInsertReducer(state, { type: "load_started", generation: 1 });
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 1,
      page: { items: [item], next_cursor: null },
    });
    state = quickInsertReducer(state, {
      type: "status",
      status: "Inserted",
      kind: "success",
    });
    state = quickInsertReducer(state, { type: "load_started", generation: 2 });
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 2,
      page: { items: [item], next_cursor: null },
    });
    expect(state.status).toBe("Inserted");
    expect(state.statusKind).toBe("success");
  });

  it("tracks search mode and preserves batch precedence in explicit state", () => {
    let state = initialQuickInsertState();
    state = quickInsertReducer(state, {
      type: "search_mode_changed",
      mode: "text-edit",
    });
    state = quickInsertReducer(state, {
      type: "history_mode_changed",
      mode: "batch",
    });
    state = quickInsertReducer(state, {
      type: "batch_selection_set",
      ids: [1, 2, 2],
    });
    expect(state.searchMode).toBe("text-edit");
    expect(state.historyMode).toBe("batch");
    expect(state.batchSelectedIds).toEqual([1, 2]);

    state = quickInsertReducer(state, {
      type: "view_changed",
      view: "favorites",
    });
    expect(state.historyMode).toBe("browse");
    expect(state.batchSelectedIds).toEqual([]);
  });
});
