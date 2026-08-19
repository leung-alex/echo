import type { Page } from "@playwright/test";

export async function installEchoFixture(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type MockItem = {
      id: number;
      source: "history" | "favorite" | "snippet";
      title: string | null;
      preview_text: string | null;
      content_type: string;
      source_app: string | null;
      updated_at: number;
      pinned: boolean;
      group_name: string | null;
      content?: string;
    };

    const state = {
      history: [
        {
          id: 1,
          source: "history" as const,
          title: null,
          preview_text: "Alpha clipboard",
          content_type: "text",
          source_app: "Echo fixture",
          updated_at: Date.now(),
          pinned: false,
          group_name: null,
        },
        {
          id: 2,
          source: "history" as const,
          title: null,
          preview_text: "Image clipboard",
          content_type: "image",
          source_app: "Echo fixture",
          updated_at: Date.now(),
          pinned: false,
          group_name: null,
        },
      ] satisfies MockItem[],
      snippets: [
        {
          id: 7,
          source: "snippet" as const,
          title: "Greeting",
          preview_text: "Hello from Echo",
          content: "Hello from Echo",
          content_type: "text",
          source_app: null,
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
        .filter(
          (item) =>
            !normalized ||
            `${item.title ?? ""} ${item.preview_text ?? ""} ${item.source_app ?? ""}`
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
          case "plugin:window|hide":
          case "plugin:window|set_focus":
          case "plugin:window|set_focusable":
          case "plugin:window|start_dragging":
            return null;
          case "activation_state":
            return null;
          case "activation_ack":
            return null;
          case "quick_insert_begin_session":
            return true;
          case "quick_insert_list":
            return listFor(String(args.view), String(args.query ?? ""));
          case "quick_insert_set_favorite": {
            const item = state.history.find(
              (candidate) => candidate.id === args.id,
            );
            if (item) item.pinned = Boolean(args.pinned);
            return Boolean(item);
          }
          case "quick_insert_delete":
            if (args.source === "snippet")
              state.snippets = state.snippets.filter(
                (item) => item.id !== args.id,
              );
            else
              state.history = state.history.filter(
                (item) => item.id !== args.id,
              );
            return true;
          case "quick_insert_execute":
            return JSON.stringify(
              args.action === "insert" ? "inserted" : "copied",
            );
          case "quick_insert_get_image":
            return args.id === 2
              ? {
                  mime_type: "image/png",
                  base64:
                    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                }
              : null;
          case "settings_get":
            return { ...state.settings };
          case "settings_update":
            Object.assign(state.settings, args.settings);
            return null;
          case "history_clear":
            state.history.length = 0;
            return null;
          case "snippets_list":
            return listFor("snippets", String(args.query ?? "")).map(
              (item) => ({
                id: item.id,
                name: item.title ?? "",
                content: item.content ?? item.preview_text ?? "",
                group_name: item.group_name,
                created_at: item.updated_at,
                updated_at: item.updated_at,
              }),
            );
          case "snippet_save": {
            const id =
              args.id === null || args.id === undefined ? 8 : Number(args.id);
            const item: MockItem = {
              id,
              source: "snippet",
              title: String(args.name),
              preview_text: String(args.content),
              content: String(args.content),
              content_type: "text",
              source_app: null,
              updated_at: Date.now(),
              pinned: false,
              group_name: args.group_name ? String(args.group_name) : null,
            };
            state.snippets = [
              ...state.snippets.filter((candidate) => candidate.id !== id),
              item,
            ];
            return id;
          }
          case "snippet_delete":
            state.snippets = state.snippets.filter(
              (item) => item.id !== args.id,
            );
            return true;
          default:
            if (command.startsWith("plugin:")) return null;
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
      __TAURI_INTERNALS__: {
        ...internals,
        metadata: { currentWindow: { label: "main" } },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => undefined },
      __echoMockState: state,
      __emitEchoActivation: (payload: unknown) => {
        callbacks.forEach((callback) =>
          callback({ event: "echo-activation", id: 1, payload }),
        );
      },
    });
  });
}
