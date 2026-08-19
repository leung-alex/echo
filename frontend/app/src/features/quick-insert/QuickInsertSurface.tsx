import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactElement,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LayoutList, Search, Settings, Table2 } from "lucide-react";

import { SearchField } from "../../ui/SearchField";
import { useActiveResultNavigation } from "../../ui/useActiveResultNavigation";
import { useQuickInsertController } from "./controller";
import {
  quickInsertClient,
  type QuickInsertClient,
} from "./api/quick-insert-client";
import { QuickInsertResults } from "./components/QuickInsertResults";
import type { PasteSession, QuickInsertView } from "./model/types";

type ClipboardViewMode = "detailed" | "compact";
const VIEW_MODE_KEY = "echo.clipboard.view-mode.v1";

function readViewMode(): ClipboardViewMode {
  try {
    const value = window.localStorage.getItem(VIEW_MODE_KEY);
    return value === "compact" || value === "detailed" ? value : "detailed";
  } catch {
    return "detailed";
  }
}

export interface QuickInsertSurfaceProps {
  initialSession?: PasteSession | null;
  initialQuery?: string;
  focusRequest?: number;
  onClose: () => void | Promise<void>;
  onOpenSettings?: () => void;
  client?: QuickInsertClient;
}

export function QuickInsertSurface({
  initialSession = null,
  initialQuery = "",
  focusRequest = 0,
  onClose,
  onOpenSettings,
  client = quickInsertClient,
}: QuickInsertSurfaceProps): ReactElement {
  const searchRef = useRef<HTMLInputElement>(null);
  const [viewMode, setViewMode] = useState<ClipboardViewMode>(readViewMode);
  const focusSearch = useCallback(() => {
    searchRef.current?.focus();
    try {
      const currentWindow = getCurrentWindow();
      void currentWindow
        .setFocusable(true)
        .then(() => currentWindow.setFocus())
        .then(() => searchRef.current?.focus())
        .catch(() => undefined);
    } catch {
      // Browser-owned tests and non-Tauri previews can still focus the input.
    }
  }, []);
  const controller = useQuickInsertController({
    initialSession,
    initialQuery,
    onClose,
    focusSearch,
    client,
  });
  const { state } = controller;
  const resultPopupId =
    state.view === "snippets" ? "echo-snippet-results" : "echo-entry-results";
  const navigation = useActiveResultNavigation({
    resultKeys: state.items.map((item) => `${item.source}:${item.id}`),
    resetToken: `${state.view}:${state.query}`,
    popupId: resultPopupId,
    popupRole: state.view === "snippets" ? "listbox" : "grid",
    visible: true,
  });

  useEffect(() => {
    if (focusRequest > 0) focusSearch();
  }, [focusRequest, focusSearch]);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const inputFocused =
      event.target instanceof HTMLInputElement ||
      event.target instanceof HTMLTextAreaElement;
    if (
      event.nativeEvent.isComposing &&
      ["ArrowUp", "ArrowDown", "Enter", "Escape"].includes(event.key)
    )
      return;
    if (event.ctrlKey && event.key.toLocaleLowerCase() === "f") {
      event.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    if (!inputFocused && event.key === "/") {
      event.preventDefault();
      searchRef.current?.focus();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      controller.handleEscape(event.nativeEvent.isComposing);
      return;
    }
    if (
      event.target === searchRef.current &&
      ["ArrowUp", "ArrowDown", "Enter"].includes(event.key)
    ) {
      event.preventDefault();
      if (event.key === "Enter" && controller.selectedItem)
        void controller.execute(controller.selectedItem);
      else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        controller.moveSelection(event.key);
        navigation.move(event.key === "ArrowUp" ? -1 : 1);
      }
      return;
    }
    if (inputFocused || event.target instanceof HTMLButtonElement) return;
    if (event.key === "Enter") {
      event.preventDefault();
      if (controller.selectedItem)
        void controller.execute(controller.selectedItem);
      return;
    }
    if (
      ["i", "j", "k", "ArrowUp", "ArrowDown"].includes(event.key) ||
      /^[1-9]$/.test(event.key)
    ) {
      event.preventDefault();
      controller.moveSelection(event.key);
    }
  };

  const selectView = (view: QuickInsertView) => {
    controller.setView(view);
    focusSearch();
  };
  const startTopbarDrag = (event: MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const target = event.target;
    if (
      target instanceof Element &&
      target.closest(
        'button, input, textarea, select, a, [contenteditable="true"], .echo-search-field',
      )
    ) {
      return;
    }
    event.preventDefault();
    void getCurrentWindow()
      .startDragging()
      .catch(() => undefined);
  };
  const selectViewMode = (mode: ClipboardViewMode) => {
    setViewMode(mode);
    try {
      window.localStorage.setItem(VIEW_MODE_KEY, mode);
    } catch {
      /* storage is optional */
    }
    focusSearch();
  };
  const isSnippetView = state.view === "snippets";

  return (
    <main
      className="clipboard-window"
      tabIndex={0}
      onKeyDown={onKeyDown}
      data-testid="clipboard-panel"
    >
      <div className="clipboard-topbar" onMouseDown={startTopbarDrag}>
        <SearchField
          ref={searchRef}
          className="clipboard-search-row"
          value={state.query}
          onChange={(event) => controller.setQuery(event.target.value)}
          placeholder={
            isSnippetView ? "Search snippets..." : "Search clipboard history..."
          }
          aria-label={
            isSnippetView ? "Search snippets" : "Search clipboard history"
          }
          autoFocus
          autoComplete="off"
          startSlot={
            <Search
              className="clipboard-search-icon"
              size={18}
              aria-hidden="true"
            />
          }
          endSlot={<kbd className="clipboard-search-shortcut">Ctrl + F</kbd>}
          {...navigation.comboboxProps}
        />
        {onOpenSettings ? (
          <button
            className="clipboard-topbar-action"
            type="button"
            aria-label="Open settings"
            title="Settings"
            onClick={onOpenSettings}
          >
            <Settings size={17} aria-hidden="true" />
          </button>
        ) : null}
      </div>
      <nav className="clipboard-tabs" aria-label="Clipboard views">
        <div className="clipboard-tab-list" role="tablist">
          {(["history", "favorites", "snippets"] as const).map((view) => (
            <button
              key={view}
              type="button"
              role="tab"
              className="clipboard-tab"
              aria-selected={state.view === view}
              aria-current={state.view === view ? "page" : undefined}
              onClick={() => selectView(view)}
            >
              {view[0].toUpperCase() + view.slice(1)}
            </button>
          ))}
        </div>
        {!isSnippetView ? (
          <div
            className="clipboard-view-toggle"
            role="group"
            aria-label="Clipboard history view"
          >
            <button
              type="button"
              aria-label="Detailed view"
              aria-pressed={viewMode === "detailed"}
              title="Detailed view"
              onClick={() => selectViewMode("detailed")}
            >
              <Table2 size={15} aria-hidden="true" />
            </button>
            <button
              type="button"
              aria-label="Compact view"
              aria-pressed={viewMode === "compact"}
              title="Compact view"
              onClick={() => selectViewMode("compact")}
            >
              <LayoutList size={16} aria-hidden="true" />
            </button>
          </div>
        ) : null}
      </nav>
      <section
        className={
          isSnippetView
            ? "clipboard-workspace"
            : `clipboard-entry-workspace clipboard-entry-workspace--${viewMode}`
        }
        aria-label={
          isSnippetView
            ? "Snippets"
            : state.view === "favorites"
              ? "Favorite entries"
              : "Clipboard entries"
        }
      >
        {isSnippetView ? (
          <div className="result-pane">
            <div className="pane-heading">
              <span>Snippets</span>
            </div>
            <QuickInsertResults
              {...resultProps(controller, navigation, client)}
              view="snippets"
              viewMode="detailed"
            />
          </div>
        ) : (
          <div className="clipboard-history-layout">
            <QuickInsertResults
              {...resultProps(controller, navigation, client)}
              view={state.view}
              viewMode={viewMode}
            />
          </div>
        )}
      </section>
      <footer className="clipboard-footer">
        <div className="key-hints" aria-label="Keyboard controls">
          <span>
            <kbd>↑↓</kbd> Navigate
          </span>
          <span>
            <kbd>Enter</kbd> Paste
          </span>
          <span>
            <kbd>/</kbd> Search
          </span>
          <span>
            <kbd>Esc</kbd> Close
          </span>
        </div>
        <output
          role={state.statusKind === "error" ? "alert" : "status"}
          aria-live={state.statusKind === "error" ? "assertive" : "polite"}
          data-kind={state.statusKind}
          data-busy={state.loading}
        >
          {state.status}
        </output>
      </footer>
    </main>
  );
}

function resultProps(
  controller: ReturnType<typeof useQuickInsertController>,
  navigation: ReturnType<typeof useActiveResultNavigation>,
  client: QuickInsertClient,
) {
  return {
    items: controller.state.items,
    query: controller.state.query,
    selected: controller.state.selection,
    emptyMessage: controller.emptyMessage,
    getResultId: (key: string) => navigation.getResultId(key),
    select: (index: number) => {
      controller.select(index);
      navigation.activateIndex(index);
    },
    execute: (
      item: Parameters<typeof controller.execute>[0],
      intent?: "insert" | "copy",
    ) => void controller.execute(item, intent),
    toggleFavorite: (item: Parameters<typeof controller.toggleFavorite>[0]) =>
      void controller.toggleFavorite(item),
    remove: (item: Parameters<typeof controller.remove>[0]) =>
      void controller.remove(item),
    getImage: client.getImage,
  };
}

export const ClipboardSurface = QuickInsertSurface;
