import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    type MockItem = {
      id: number;
      source: "history" | "favorite" | "snippet";
      title?: string;
      preview_text?: string;
      content_type: string;
      source_app?: string;
      updated_at: number;
      pinned: boolean;
      group_name?: string;
    };

    const state = {
      history: [
        {
          id: 1,
          source: "history" as const,
          preview_text: "Alpha clipboard",
          content_type: "text",
          source_app: "Echo fixture",
          updated_at: Date.now(),
          pinned: false,
        },
      ] satisfies MockItem[],
      snippets: [
        {
          id: 7,
          source: "snippet" as const,
          title: "Greeting",
          preview_text: "Hello from Echo",
          content_type: "snippet",
          updated_at: Date.now(),
          pinned: false,
          group_name: "Common",
        },
      ] satisfies MockItem[],
      settings: {
        history_enabled: true,
        record_sensitive: false,
        store_window_titles: true,
        max_entries: 500,
        max_total_bytes: 50_000_000,
        max_item_bytes: 5_000_000,
      },
      calls: [] as Array<{ command: string; args: Record<string, unknown> }>,
    };

    const callbacks = new Map<number, (event: unknown) => void>();
    let nextCallback = 1;
    const listFor = (view: string, query: string): MockItem[] => {
      const source =
        view === "favorites"
          ? state.history
              .filter((item) => item.pinned)
              .map((item) => ({ ...item, source: "favorite" as const }))
          : view === "snippets"
            ? state.snippets
            : state.history;
      const normalized = query.trim().toLowerCase();
      return source
        .filter((item) =>
          !normalized
            ? true
            : `${item.title ?? ""} ${item.preview_text ?? ""}`
                .toLowerCase()
                .includes(normalized),
        )
        .map((item) => ({ ...item }));
    };

    const internals = {
      invoke: async (command: string, args: Record<string, unknown> = {}) => {
        state.calls.push({ command, args });
        switch (command) {
          case "plugin:event|listen":
          case "plugin:event|unlisten":
            return null;
          case "activation_state":
            return null;
          case "activation_ack":
            return null;
          case "quick_insert_list":
            return listFor(String(args.view), String(args.query ?? ""));
          case "quick_insert_set_favorite": {
            const item = state.history.find(
              (candidate) => candidate.id === args.id,
            );
            if (item) item.pinned = Boolean(args.pinned);
            return Boolean(item);
          }
          case "quick_insert_execute":
            return JSON.stringify(
              args.action === "insert" ? "inserted" : "copied",
            );
          case "settings_get":
            return { ...state.settings };
          case "settings_update":
            Object.assign(state.settings, args.settings);
            return null;
          case "history_clear":
            state.history.length = 0;
            return null;
          case "snippet_save": {
            const item: MockItem = {
              id: 8,
              source: "snippet",
              title: String(args.name),
              preview_text: String(args.content),
              content_type: "snippet",
              updated_at: Date.now(),
              pinned: false,
              group_name: args.groupName ? String(args.groupName) : undefined,
            };
            state.snippets = [
              ...state.snippets.filter((candidate) => candidate.id !== args.id),
              item,
            ];
            return item.id;
          }
          default:
            throw new Error(`Unexpected Echo mock command: ${command}`);
        }
      },
      transformCallback: (callback: (event: unknown) => void) => {
        const id = nextCallback++;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => {
        callbacks.delete(id);
      },
    };

    Object.assign(window, {
      __TAURI_INTERNALS__: internals,
      __echoMockState: state,
    });
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "History" })).toBeVisible();
  await expect(page.getByText("Alpha clipboard")).toBeVisible();
});

test("keeps History, Favorites, Snippets, copy, search, and settings on one Echo surface", async ({
  page,
}) => {
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("No favorites yet")).toBeVisible();

  await page.getByRole("tab", { name: "History" }).click();
  await page.getByRole("button", { name: "Favorite" }).click();
  await expect(page.getByText("Added to Favorites")).toBeVisible();
  await page.getByRole("tab", { name: "Favorites" }).click();
  await expect(page.getByText("Alpha clipboard")).toBeVisible();

  await page.getByRole("tab", { name: "Snippets" }).click();
  await expect(page.getByText("Hello from Echo")).toBeVisible();
  await page.getByRole("button", { name: "New snippet" }).click();
  await page.locator('input[name="name"]').fill("Saved from UI");
  await page.locator('textarea[name="content"]').fill("UI snippet body");
  await page.getByRole("button", { name: "Save snippet" }).click();
  await expect(page.getByText("UI snippet body")).toBeVisible();
  await page.getByPlaceholder("Search snippets").fill("UI snippet");
  await expect(page.getByText("UI snippet body")).toBeVisible();

  await page.getByRole("tab", { name: "History" }).click();
  await page.getByRole("button", { name: "Copy" }).click();
  await expect(page.getByText("Copied")).toBeVisible();
  await page.getByRole("button", { name: "Open settings" }).click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await page.getByLabel("Record sensitive content").check();
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByText("Settings updated")).toBeVisible();
});

test("uses escaped user content and keeps action buttons scoped to their item", async ({
  page,
}) => {
  await page.getByRole("tab", { name: "History" }).click();
  const item = page.locator('[data-id="1"][data-source="history"]');
  await expect(item.getByRole("button", { name: "Copy" })).toHaveCount(1);
  await expect(item.locator("script")).toHaveCount(0);
});
