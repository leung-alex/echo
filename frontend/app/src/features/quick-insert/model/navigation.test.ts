import { describe, expect, it } from "vitest";

import { activeResultId, nextSelection, selectionForKey } from "./navigation";

describe("quick insert navigation", () => {
  it("clamps keyboard movement and numeric selection", () => {
    expect(nextSelection(-1, 3, 1)).toBe(0);
    expect(nextSelection(2, 3, 1)).toBe(2);
    expect(nextSelection(0, 3, -1)).toBe(0);
    expect(selectionForKey("2", 3)).toBe(1);
    expect(selectionForKey("9", 3)).toBeNull();
  });

  it("encodes result ids without unsafe punctuation", () => {
    expect(activeResultId("echo-results", "history:4_5")).toBe(
      "echo-results-result-history_3A4_5F5",
    );
  });
});
