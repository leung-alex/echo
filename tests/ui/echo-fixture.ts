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
      pinned_at: number | null;
      icon_key: string | null;
      favorite_order: number | null;
      preview: {
        url: string;
        mime_type: string;
        width: number;
        height: number;
        byte_size: number;
        content_hash: string;
      } | null;
    };

    const favoritesStorageKey = "echo-ui-fixture-favorites-v1";
    const storedFavorites = (() => {
      try {
        const raw = window.localStorage.getItem(favoritesStorageKey);
        const parsed: unknown = raw ? JSON.parse(raw) : null;
        return Array.isArray(parsed) ? (parsed as MockItem[]) : [];
      } catch {
        return [];
      }
    })();

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
          pinned_at: null,
          icon_key: null,
          favorite_order: null,
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
          pinned_at: null,
          icon_key: null,
          favorite_order: null,
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
      favorites: storedFavorites,
      activePanel: "history" as const,
      nextFavoriteId: 100,
      settings: {
        history_enabled: true,
        record_sensitive: false,
        store_window_titles: true,
        max_entries: 500,
        max_total_bytes: 50_000_000,
        max_item_bytes: 5_000_000,
        theme: "system" as const,
      },
      calls: [] as Array<{ command: string; args: Record<string, unknown> }>,
    };
    const callbacks = new Map<number, (event: unknown) => void>();
    const callbackEvents = new Map<number, string>();
    let nextCallback = 1;

    const emit = (event: string, payload: unknown) => {
      callbacks.forEach((callback, id) => {
        if (callbackEvents.get(id) === event) {
          callback({ event, id: 1, payload });
        }
      });
    };
    const emitLibraryChanged = (
      kind: "history" | "favorites" | "history_and_favorites",
    ) => emit("echo-library-changed", { version: Date.now(), kind });
    const emitHistoryChanged = () =>
      emitLibraryChanged("history_and_favorites");
    const normalizeFavoriteOrder = () => {
      state.favorites = state.favorites.map((item, index) => ({
        ...item,
        favorite_order: index,
      }));
    };
    const persistFavorites = () => {
      try {
        window.localStorage.setItem(
          favoritesStorageKey,
          JSON.stringify(state.favorites),
        );
      } catch {
        // The fixture remains usable in an opaque browser context.
      }
    };
    const findFavorite = (id: unknown) =>
      state.favorites.find((candidate) => candidate.id === id);

    const moveHistoryToFavorite = (id: number): MockItem => {
      const item = state.history.find((candidate) => candidate.id === id);
      if (!item) throw new Error(`History item ${id} was not found`);
      const favorite: MockItem = {
        ...item,
        source: "favorite",
        name: item.name ?? item.preview_text,
        pinned_at: null,
        icon_key: item.id === 1 ? "StarFilled" : null,
        favorite_order: 0,
      };
      state.history = state.history.filter((candidate) => candidate.id !== id);
      state.favorites.unshift(favorite);
      normalizeFavoriteOrder();
      persistFavorites();
      emitLibraryChanged("history_and_favorites");
      return { ...favorite };
    };

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
        .map((item) => ({ ...item, tags: [...item.tags] }));
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
          case "activation_ack":
          case "quick_insert_clear_session":
            return null;
          case "quick_insert_begin_session":
            return { hasTarget: true };
          case "quick_insert_active_panel":
            return state.activePanel;
          case "quick_insert_activate_panel":
            state.activePanel = args.panel as "history" | "favorites";
            emit("echo-active-panel-changed", { panel: state.activePanel });
            return null;
          case "quick_insert_list":
            return {
              items: listFor(String(args.view), String(args.query ?? "")),
              next_cursor: null,
            };
          case "quick_insert_move_history_to_favorite":
            return moveHistoryToFavorite(Number(args.id));
          case "quick_insert_move_history_many_to_favorites": {
            const request = args.request as { ids?: number[] } | undefined;
            return (request?.ids ?? []).map((id) =>
              moveHistoryToFavorite(Number(id)),
            );
          }
          case "quick_insert_create_favorite": {
            const draft = args.draft as {
              content: string;
              name: string | null;
              icon_key: string | null;
              tags: string[];
            };
            const favorite: MockItem = {
              id: state.nextFavoriteId++,
              source: "favorite",
              name: (draft.name ?? draft.content.slice(0, 48)) || "Favorite",
              preview_text: draft.content,
              content_type: "text",
              editable_text: draft.content,
              tags: [...draft.tags],
              source_app: "Echo fixture",
              updated_at: Date.now(),
              pinned_at: null,
              icon_key: draft.icon_key,
              favorite_order: 0,
              preview: null,
            };
            state.favorites.unshift(favorite);
            normalizeFavoriteOrder();
            persistFavorites();
            emitLibraryChanged("favorites");
            return { ...favorite };
          }
          case "quick_insert_update_favorite": {
            const update = args.update as {
              name: string | null;
              icon_key: string | null;
              tags: string[];
              editable_text: string | null;
            };
            const item = findFavorite(Number(args.id));
            if (!item)
              throw new Error(`Favorite ${String(args.id)} was not found`);
            item.name = update.name ?? item.name;
            item.icon_key = update.icon_key;
            item.tags = [...update.tags];
            if (item.editable_text !== null && update.editable_text !== null) {
              item.editable_text = update.editable_text;
              item.preview_text = update.editable_text;
            }
            persistFavorites();
            emitLibraryChanged("favorites");
            return { ...item };
          }
          case "quick_insert_pin_history": {
            const item = state.history.find(
              (candidate) => candidate.id === Number(args.id),
            );
            if (!item) return false;
            item.pinned_at = Date.now();
            emitLibraryChanged("history");
            return true;
          }
          case "quick_insert_unpin_history": {
            const item = state.history.find(
              (candidate) => candidate.id === Number(args.id),
            );
            if (!item) return false;
            item.pinned_at = null;
            emitLibraryChanged("history");
            return true;
          }
          case "quick_insert_pin_history_many": {
            const request = args.request as { ids?: number[] } | undefined;
            const ids = new Set(request?.ids ?? []);
            state.history.forEach((item) => {
              if (ids.has(item.id)) item.pinned_at = Date.now();
            });
            emitLibraryChanged("history");
            return ids.size;
          }
          case "quick_insert_delete_history_many": {
            const request = args.request as { ids?: number[] } | undefined;
            const ids = new Set(request?.ids ?? []);
            const before = state.history.length;
            state.history = state.history.filter((item) => !ids.has(item.id));
            emitLibraryChanged("history");
            return before - state.history.length;
          }
          case "quick_insert_clear_unpinned_history": {
            const before = state.history.length;
            state.history = state.history.filter(
              (item) => item.pinned_at !== null,
            );
            emitLibraryChanged("history");
            return before - state.history.length;
          }
          case "quick_insert_reorder_favorites": {
            const request = args.request as
              | { ordered_ids?: number[] }
              | undefined;
            const order = request?.ordered_ids ?? [];
            const byId = new Map(
              state.favorites.map((item) => [item.id, item]),
            );
            state.favorites = [
              ...order.flatMap((id) => {
                const item = byId.get(id);
                return item ? [item] : [];
              }),
              ...state.favorites.filter((item) => !order.includes(item.id)),
            ];
            normalizeFavoriteOrder();
            persistFavorites();
            emitLibraryChanged("favorites");
            return null;
          }
          case "quick_insert_delete_favorite": {
            const before = state.favorites.length;
            state.favorites = state.favorites.filter(
              (item) => item.id !== Number(args.id),
            );
            normalizeFavoriteOrder();
            persistFavorites();
            emitLibraryChanged("favorites");
            return before !== state.favorites.length;
          }
          case "quick_insert_execute":
            return args.action === "insert" ? "inserted" : "copied";
          case "settings_get":
            return { ...state.settings };
          case "settings_update": {
            Object.assign(state.settings, args.settings);
            emit("echo-theme-changed", {
              mode: state.settings.theme,
              nativeMica: false,
            });
            return null;
          }
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
    const windowRole =
      new URL(window.location.href).searchParams.get("windowRole") ===
      "favorites"
        ? "favorites"
        : "main";
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        ...internals,
        metadata: { currentWindow: { label: windowRole } },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => undefined },
      __echoMockState: state,
      __emitEchoActivation: (payload: unknown) =>
        emit("echo-activation", payload),
      __emitEchoThemeChanged: (payload: unknown) =>
        emit("echo-theme-changed", payload),
      __emitEchoHistoryChanged: () => emitHistoryChanged(),
    });
  });
}
