import type { Page } from "@playwright/test";

export async function installEchoFixture(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type MockItem = {
      id: number;
      source: "history" | "favorite";
      title: string | null;
      preview_text: string | null;
      content_type: string;
      source_app: string | null;
      updated_at: number;
      pinned: boolean;
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
            return { hasTarget: true };
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
            state.history = state.history.filter((item) => item.id !== args.id);
            return true;
          case "quick_insert_execute":
            return args.action === "insert" ? "inserted" : "copied";
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
