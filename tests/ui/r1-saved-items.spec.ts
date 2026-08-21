import { expect, test } from "@playwright/test";

import { installEchoFixture } from "./echo-fixture";

test("renders Favorites as direct-action rows with persistent-order affordances", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();

  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("tab", { name: "Favorites" }).click();

  await expect(page.getByRole("row")).toHaveCount(2);
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await expect(
    page.getByText("Manual order · drag to rearrange"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Edit saved item" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Delete" }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Unfavorite|Favorite/ }),
  ).toHaveCount(0);

  await page.getByRole("row").nth(1).dragTo(page.getByRole("row").first());
  await expect(page.getByRole("row").first()).toContainText("Alpha clipboard");

  await page
    .getByRole("combobox", { name: "Search clipboard history" })
    .fill("Alpha");
  await expect(
    page.getByText("Search relevance · drag reorder disabled"),
  ).toBeVisible();
  await expect(page.locator('[draggable="true"]')).toHaveCount(0);
  await expect(page.locator(".echo-search-match").first()).toContainText(
    "Alpha",
  );
});

test("opens the create path and Apps SDK icon picker with keyboard affordances", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("button", { name: "Create favorite" }).click();

  const editor = page.getByRole("dialog", { name: "Create favorite" });
  await expect(editor).toContainText("Content");
  await expect(editor).toContainText("Name");
  await expect(editor).toContainText("Icon");
  await expect(editor).toContainText("Tags");
  await editor.getByRole("button", { name: "Choose favorite icon" }).click();

  const picker = page.getByRole("dialog", { name: "Choose favorite icon" });
  await expect(picker).toBeVisible();
  await expect(picker.getByRole("option", { name: "No icon" })).toBeVisible();
  await picker
    .getByPlaceholder("Search the full icon catalog")
    .fill("terminal");
  await expect(
    picker.getByRole("option", { name: "Terminal", exact: true }),
  ).toBeVisible();
  await picker
    .getByPlaceholder("Search the full icon catalog")
    .press("ArrowDown");
  await picker.getByPlaceholder("Search the full icon catalog").press("Enter");
  await expect(editor.getByText("Terminal")).toBeVisible();
});

test("requires non-empty Favorite Content with an accessible focused error", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await page.getByRole("tab", { name: "Favorites" }).click();
  await page.getByRole("button", { name: "Create favorite" }).click();

  const editor = page.getByRole("dialog", { name: "Create favorite" });
  const content = editor.getByRole("textbox", { name: "Content" });
  await expect(content).toHaveAttribute("aria-required", "true");
  await content.fill("   ");
  await editor.getByRole("button", { name: "Save Favorite" }).click();

  await expect(content).toHaveAttribute("aria-invalid", "true");
  await expect(content).toHaveAttribute(
    "aria-describedby",
    "favorite-content-error",
  );
  await expect(editor.getByRole("alert")).toHaveText("Content is required.");
  await expect(content).toBeFocused();
  await content.fill("Reusable content");
  await expect(content).not.toHaveAttribute("aria-invalid", "true");
  await editor.getByRole("button", { name: "Save Favorite" }).click();
  await expect(editor).toBeHidden();
});
