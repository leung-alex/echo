import { createElement, createRef } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { SearchField, shouldShowCustomCaret } from "./SearchField";

const visibleCaret = {
  focused: true,
  composing: false,
  selectionStart: 3,
  selectionEnd: 3,
  caretLeft: 80,
  inputLeft: 40,
  inputRight: 320,
};

describe("SearchField", () => {
  it("renders the measured caret layer with product-owned slots", () => {
    const html = renderToStaticMarkup(
      createElement(SearchField, {
        ref: createRef<HTMLInputElement>(),
        value: "",
        readOnly: true,
        "aria-label": "Search",
        startSlot: createElement("span", null, "Search icon"),
        endSlot: createElement("kbd", null, "Ctrl + F"),
      }),
    );

    expect(html.match(/<input/g)).toHaveLength(1);
    expect(html).toContain("Search icon");
    expect(html).toContain("Ctrl + F");
    expect(html).toContain("echo-search-field__measure");
    expect(html).toContain("echo-search-field__caret");
  });

  it("shows the custom caret for a focused collapsed selection", () => {
    expect(shouldShowCustomCaret(visibleCaret)).toBe(true);
  });

  it("uses the native caret while composing text", () => {
    expect(shouldShowCustomCaret({ ...visibleCaret, composing: true })).toBe(
      false,
    );
  });

  it("hides the custom caret for a text selection or blur", () => {
    expect(shouldShowCustomCaret({ ...visibleCaret, selectionEnd: 7 })).toBe(
      false,
    );
    expect(shouldShowCustomCaret({ ...visibleCaret, focused: false })).toBe(
      false,
    );
  });

  it("hides the custom caret when horizontal scrolling clips it", () => {
    expect(shouldShowCustomCaret({ ...visibleCaret, caretLeft: 38 })).toBe(
      false,
    );
    expect(shouldShowCustomCaret({ ...visibleCaret, caretLeft: 319 })).toBe(
      false,
    );
  });
});
