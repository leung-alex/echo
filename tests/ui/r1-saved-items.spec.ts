import { expect, test } from "@playwright/test";

import { installEchoFixture } from "./echo-fixture";

test("retains bulk-delete selection when the backend rejects the transaction", async ({
  page,
}) => {
  await installEchoFixture(page);
  await page.goto("/");
  await expect(page.getByText("Alpha clipboard")).toBeVisible();

  await page.getByRole("button", { name: "Favorite" }).first().click();
  await page.getByRole("button", { name: "Favorite" }).nth(1).click();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByRole("row")).toHaveCount(2);

  const selections = page.getByRole("checkbox");
  await expect(selections).toHaveCount(2);
  await selections.nth(0).check();
  await selections.nth(1).check();
  await expect(
    page.getByRole("button", { name: "Delete selected" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Delete selected" }).click();

  await expect(
    page.getByRole("button", { name: "Delete selected" }),
  ).toBeVisible();
  await expect(selections.nth(0)).toBeChecked();
  await expect(selections.nth(1)).toBeChecked();
  await expect(page.getByRole("row")).toHaveCount(2);
});
