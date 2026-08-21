import { describe, expect, it } from "vitest";

import { interpretGlobalKey } from "./interaction";

const base = {
  searchMode: "navigation" as const,
  historyMode: "browse" as const,
  view: "history" as const,
  target: "surface" as const,
  composing: false,
};

describe("quick insert interaction state machine", () => {
  it("keeps initial typing in navigation mode while reserving explicit text editing for Ctrl+F", () => {
    expect(interpretGlobalKey({ ...base, key: "a", target: "search" })).toEqual(
      { type: "none" },
    );
    expect(interpretGlobalKey({ ...base, key: "f", ctrlKey: true })).toEqual({
      type: "focus-search",
      select: true,
    });
  });

  it("switches panels with Tab and horizontal control chords", () => {
    expect(interpretGlobalKey({ ...base, key: "Tab" })).toEqual({
      type: "switch-panel",
      direction: 1,
    });
    expect(interpretGlobalKey({ ...base, key: "h", ctrlKey: true })).toEqual({
      type: "switch-panel",
      direction: -1,
    });
    expect(interpretGlobalKey({ ...base, key: "l", ctrlKey: true })).toEqual({
      type: "switch-panel",
      direction: 1,
    });
  });

  it("gives batch selection precedence to Space, Enter, and Ctrl+A", () => {
    const batch = { ...base, historyMode: "batch" as const };
    expect(interpretGlobalKey({ ...batch, key: " " })).toEqual({
      type: "toggle-batch-selection",
    });
    expect(interpretGlobalKey({ ...batch, key: "Enter" })).toEqual({
      type: "toggle-batch-selection",
    });
    expect(interpretGlobalKey({ ...batch, key: "a", ctrlKey: true })).toEqual({
      type: "select-all-batch",
    });
  });

  it("leaves Tab traversal available inside result rows and action controls", () => {
    expect(interpretGlobalKey({ ...base, key: "Tab", target: "row" })).toEqual({
      type: "none",
    });
    expect(
      interpretGlobalKey({ ...base, key: "Tab", target: "control" }),
    ).toEqual({ type: "none" });
  });

  it("keeps horizontal caret editing native in Text Edit mode", () => {
    expect(
      interpretGlobalKey({
        ...base,
        key: "ArrowLeft",
        target: "search",
        searchMode: "text-edit",
      }),
    ).toEqual({ type: "none" });
    expect(
      interpretGlobalKey({ ...base, key: "Escape", target: "search" }),
    ).toEqual({ type: "close" });
  });

  it("keeps vertical navigation and Tab panel switching available in Text Edit mode", () => {
    const textEdit = {
      ...base,
      searchMode: "text-edit" as const,
      target: "search" as const,
    };
    expect(interpretGlobalKey({ ...textEdit, key: "ArrowDown" })).toEqual({
      type: "move-selection",
      direction: 1,
      key: "ArrowDown",
    });
    expect(interpretGlobalKey({ ...textEdit, key: "Tab" })).toEqual({
      type: "switch-panel",
      direction: 1,
    });
  });
});
