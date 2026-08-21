export type SearchMode = "navigation" | "text-edit";
export type HistoryMode = "browse" | "batch";
export type InteractionTarget = "search" | "row" | "control" | "surface";

export type GlobalKeyIntent =
  | { type: "none" }
  | { type: "close" }
  | { type: "focus-search"; select: boolean }
  | { type: "switch-panel"; direction: -1 | 1 }
  | { type: "move-selection"; direction: -1 | 1; key: string }
  | { type: "toggle-batch-selection" }
  | { type: "select-all-batch" }
  | { type: "primary-action" }
  | { type: "prevent-default" };

export function interpretGlobalKey({
  key,
  ctrlKey = false,
  shiftKey = false,
  searchMode,
  historyMode,
  view,
  target,
  composing,
}: {
  key: string;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  searchMode: SearchMode;
  historyMode: HistoryMode;
  view: "history" | "favorites";
  target: InteractionTarget;
  composing: boolean;
}): GlobalKeyIntent {
  if (composing) return { type: "none" };

  const lowerKey = key.toLocaleLowerCase();
  const isHistoryBatch = view === "history" && historyMode === "batch";

  if (ctrlKey && lowerKey === "f") {
    return { type: "focus-search", select: true };
  }

  if (key === "Escape") return { type: "close" };

  if (isHistoryBatch) {
    if (ctrlKey && lowerKey === "a") {
      return { type: "select-all-batch" };
    }
    if (key === " " || key === "Enter") {
      return { type: "toggle-batch-selection" };
    }
  }

  if (key === "Tab" && target !== "row" && target !== "control") {
    return { type: "switch-panel", direction: shiftKey ? -1 : 1 };
  }

  if (searchMode === "navigation" && ctrlKey) {
    if (lowerKey === "h") return { type: "switch-panel", direction: -1 };
    if (lowerKey === "l") return { type: "switch-panel", direction: 1 };
  }

  if (ctrlKey && lowerKey === "j") {
    return { type: "move-selection", direction: 1, key };
  }
  if (ctrlKey && lowerKey === "k") {
    return { type: "move-selection", direction: -1, key };
  }

  if (key === "ArrowUp" || key === "k") {
    if (target !== "search" || key === "ArrowUp") {
      return { type: "move-selection", direction: -1, key };
    }
  }
  if (key === "ArrowDown" || key === "j" || key === "i") {
    if (target !== "search" || key === "ArrowDown") {
      return { type: "move-selection", direction: 1, key };
    }
  }

  if (target === "control") return { type: "none" };

  if (key === "Enter") return { type: "primary-action" };

  if (target !== "search" && /^[1-9]$/.test(key)) {
    return { type: "move-selection", direction: 1, key };
  }

  if (key === "/" && target !== "search") {
    return { type: "focus-search", select: false };
  }

  if (target === "search" && searchMode === "navigation") {
    if (
      key === "ArrowLeft" ||
      key === "ArrowRight" ||
      key === "Home" ||
      key === "End" ||
      (ctrlKey && lowerKey === "a")
    ) {
      return { type: "prevent-default" };
    }
  }

  return { type: "none" };
}
