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
  saved_item_id: null,
  is_independent: false,
  preview: null,
  group_name: null,
};

describe("quick insert reducer", () => {
  it("keeps only the newest async load", () => {
    let state = initialQuickInsertState();
    state = quickInsertReducer(state, { type: "load_started", generation: 1 });
    state = quickInsertReducer(state, { type: "load_started", generation: 2 });
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 1,
      items: [item],
    });
    expect(state.items).toEqual([]);
    state = quickInsertReducer(state, {
      type: "load_succeeded",
      generation: 2,
      items: [item],
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
      items: [item],
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
      items: [item],
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
      items: [item],
    });
    expect(state.status).toBe("Inserted");
    expect(state.statusKind).toBe("success");
  });
});
