import type { Page } from "@playwright/test";

export async function installEchoFixture(page: Page): Promise<void> {
  await page.addInitScript(() => {
    type MockItem = {
      id: number;
      source: "history" | "favorite";
      name: string | null;
      preview_text: string | null;
      content_type: string;
      editable_text: string | null;
      tags: string[];
      source_app: string | null;
      updated_at: number;
      saved_item_id: number | null;
      is_independent: boolean;
      icon_key?: string | null;
      preview: {
        url: string;
        mime_type: string;
        width: number;
        height: number;
        byte_size: number;
        content_hash: string;
      } | null;
    };

    const state = {
      history: [
        {
          id: 1,
          source: "history" as const,
          name: null,
          preview_text: "Alpha clipboard",
          content_type: "text",
          editable_text: "Alpha clipboard",
          tags: [],
          source_app: "Echo fixture",
          updated_at: Date.now(),
          saved_item_id: null,
          is_independent: false,
          preview: null,
        },
        {
          id: 2,
          source: "history" as const,
          name: null,
          preview_text: "Image clipboard",
          content_type: "image",
          editable_text: null,
          tags: [],
          source_app: "Echo fixture",
          updated_at: Date.now(),
          saved_item_id: null,
          is_independent: false,
          preview: {
            url: "/echo-fixture.svg",
            mime_type: "image/svg+xml",
            width: 640,
            height: 360,
            byte_size: 512,
            content_hash: "fixture-image-v1",
          },
        },
      ] satisfies MockItem[],
      favorites: [] as MockItem[],
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
    const callbackEvents = new Map<number, string>();
    let nextCallback = 1;
    const listFor = (view: string, query: string): MockItem[] => {
      const source = view === "favorites" ? state.favorites : state.history;
      const normalized = query.trim().toLowerCase();
      return source
        .filter(
          (item) =>
            !normalized ||
            `${item.name ?? ""} ${item.preview_text ?? ""} ${item.source_app ?? ""} ${item.tags.join(" ")}`
              .toLowerCase()
              .includes(normalized),
        )
        .map((item) => ({ ...item }));
    };
    const emitHistoryChanged = () => {
      callbacks.forEach((callback, id) => {
        if (callbackEvents.get(id) === "echo-history-changed") {
          callback({
            event: "echo-history-changed",
            id: 1,
            payload: { version: Date.now() },
          });
        }
      });
    };
    const internals = {
      invoke: async (command: string, args: Record<string, unknown> = {}) => {
        state.calls.push({ command, args });
        switch (command) {
          case "plugin:event|listen":
            callbackEvents.set(Number(args.handler), String(args.event));
            return null;
          case "plugin:event|unlisten":
            callbackEvents.delete(Number(args.handler));
            return null;
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
            return {
              items: listFor(String(args.view), String(args.query ?? "")),
              next_cursor: null,
            };
          case "quick_insert_set_favorite": {
            const item = state.history.find(
              (candidate) => candidate.id === args.id,
            );
            if (item && Boolean(args.saved)) {
              const favorite = {
                ...item,
                source: "favorite" as const,
                name: item.preview_text,
                saved_item_id: item.id,
                is_independent: true,
                icon_key: item.id === 1 ? "StarFilled" : null,
              } satisfies MockItem;
              state.history = state.history.filter(
                (candidate) => candidate.id !== item.id,
              );
              state.favorites.unshift(favorite);
              emitHistoryChanged();
            }
            return Boolean(item);
          }
          case "quick_insert_delete":
            if (args.source === "favorite") {
              state.favorites = state.favorites.filter(
                (candidate) => candidate.id !== args.id,
              );
            } else {
              state.history = state.history.filter(
                (item) => item.id !== args.id,
              );
            }
            emitHistoryChanged();
            return true;
          case "saved_item_update": {
            const update = args.update as {
              name?: string;
              tags?: string[];
              editable_text?: string | null;
            };
            const item = state.favorites.find(
              (candidate) => candidate.id === args.id,
            );
            if (item) {
              item.name = update.name ?? item.name;
              item.tags = update.tags ?? item.tags;
              if (
                item.editable_text !== null &&
                update.editable_text !== undefined
              ) {
                item.editable_text = update.editable_text;
                item.preview_text = update.editable_text;
              }
            }
            emitHistoryChanged();
            return null;
          }
          case "saved_items_delete_many": {
            const ids = new Set((args.ids as number[]) ?? []);
            const before = state.favorites.length;
            state.favorites = state.favorites.filter(
              (item) => !ids.has(item.id),
            );
            emitHistoryChanged();
            return before - state.favorites.length;
          }
          case "quick_insert_execute":
            return args.action === "insert" ? "inserted" : "copied";
          case "settings_get":
            return { ...state.settings };
          case "settings_update":
            Object.assign(state.settings, args.settings);
            return null;
          case "history_clear":
            state.history.length = 0;
            emitHistoryChanged();
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
        callbackEvents.delete(id);
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
        callbacks.forEach((callback, id) => {
          if (callbackEvents.get(id) === "echo-activation") {
            callback({ event: "echo-activation", id: 1, payload });
          }
        });
      },
      __emitEchoHistoryChanged: () => emitHistoryChanged(),
    });
  });
}
