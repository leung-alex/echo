import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { EchoIcon, FavoriteIcon } from "./EchoIcon";
import {
  CURATED_FAVORITE_ICON_KEYS,
  FAVORITE_ICON_KEYS,
  getFavoriteIconComponent,
  NO_ICON_KEY,
} from "./favorite-icon-catalog";
import { getEchoIcon } from "./registry";

describe("Echo icon registry", () => {
  it("maps semantic names to verified Apps SDK components", () => {
    expect(getEchoIcon("search")).not.toBeNull();
    expect(getEchoIcon("settings")).not.toBeNull();
    expect(getEchoIcon("viewDetailed")).not.toBeNull();
    expect(getEchoIcon("unknown")).toBeNull();
  });

  it("exposes No icon first and keeps the public catalog searchable", () => {
    expect(FAVORITE_ICON_KEYS[0]).toBe(NO_ICON_KEY);
    expect(CURATED_FAVORITE_ICON_KEYS).toContain("Clipboard");
    expect(FAVORITE_ICON_KEYS).toContain("StarFilled");
    expect(getFavoriteIconComponent("Mail")).not.toBeNull();
  });

  it("renders unknown and No icon keys without a placeholder or gap", () => {
    const unknown = renderToStaticMarkup(
      <span data-testid="slot">
        <FavoriteIcon iconKey="removed-after-upgrade" />
      </span>,
    );
    const none = renderToStaticMarkup(
      <span data-testid="slot">
        <FavoriteIcon iconKey={NO_ICON_KEY} />
      </span>,
    );

    expect(unknown).toBe('<span data-testid="slot"></span>');
    expect(none).toBe('<span data-testid="slot"></span>');
  });

  it("adds the shared icon class to rendered semantic icons", () => {
    const markup = renderToStaticMarkup(
      <EchoIcon name="search" aria-hidden="true" />,
    );
    expect(markup).toContain('class="echo-icon"');
  });
});
