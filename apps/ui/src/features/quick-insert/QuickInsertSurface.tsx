import {
  useCallback,
  useEffect,
  useRef,
  type FocusEvent,
  type KeyboardEvent,
  type ReactElement,
  type RefObject,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { EchoIcon } from "../../ui/icons/EchoIcon";
import { SearchField } from "../../ui/SearchField";
import { useActiveResultNavigation } from "../../ui/useActiveResultNavigation";
import { HistoryResults } from "../history/HistoryResults";
import { SavedItemsResults } from "../saved-items/SavedItemsResults";
import { useQuickInsertController } from "./controller";
import {
  quickInsertClient,
  type QuickInsertClient,
} from "./api/quick-insert-client";
import {
  interpretGlobalKey,
  type InteractionTarget,
} from "./model/interaction";
import type {
  PasteSession,
  QuickInsertView,
  RuntimeContext,
  WindowRole,
} from "./model/types";

export interface QuickInsertSurfaceProps {
  initialSession?: PasteSession | null;
  initialView?: QuickInsertView;
  initialQuery?: string;
  focusRequest?: number;
  runtimeContext?: RuntimeContext;
  windowRole?: WindowRole;
  showPanelTabs?: boolean;
  onClose: () => void | Promise<void>;
  onOpenSettings?: () => void;
  client?: QuickInsertClient;
}

export function QuickInsertSurface({
  initialSession = null,
  initialView = "history",
  initialQuery = "",
  focusRequest = 0,
  runtimeContext = "manager",
  windowRole = "main",
  showPanelTabs = windowRole === "main",
  onClose,
  onOpenSettings,
  client = quickInsertClient,
}: QuickInsertSurfaceProps): ReactElement {
  const searchRef = useRef<HTMLInputElement>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const focusSearch = useCallback((select = false) => {
    const input = searchRef.current;
    input?.focus();
    if (select) input?.select();
    try {
      const currentWindow = getCurrentWindow();
      void currentWindow
        .setFocusable(true)
        .then(() => currentWindow.setFocus())
        .then(() => {
          input?.focus();
          if (select) input?.select();
        })
        .catch(() => undefined);
    } catch {
      // Browser-owned tests and non-Tauri previews can still focus the input.
    }
  }, []);
  const controller = useQuickInsertController({
    initialSession,
    initialView,
    initialQuery,
    runtimeContext,
    windowRole,
    onClose,
    focusSearch,
    client,
  });
  const { state } = controller;
  const resultPopupId = "echo-entry-results";
  const navigation = useActiveResultNavigation({
    resultKeys: state.items.map((item) => `${item.source}:${item.id}`),
    resetToken: `${state.view}:${state.query}`,
    popupId: resultPopupId,
    popupRole: "grid",
    visible: true,
  });

  useEffect(() => {
    if (focusRequest > 0) focusSearch();
  }, [focusRequest, focusSearch]);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target;
    const interactionTarget: InteractionTarget =
      target === searchRef.current
        ? "search"
        : target instanceof Element && target.closest('[role="row"]')
          ? "row"
          : target instanceof Element &&
              (target.closest("button, input, textarea, select") ||
                target.closest('[role="dialog"]'))
            ? "control"
            : "surface";
    const intent = interpretGlobalKey({
      key: event.key,
      ctrlKey: event.ctrlKey,
      shiftKey: event.shiftKey,
      searchMode: state.searchMode,
      historyMode: state.historyMode,
      view: state.view,
      target: interactionTarget,
      composing: event.nativeEvent.isComposing,
    });

    if (intent.type === "none") return;
    event.preventDefault();

    switch (intent.type) {
      case "close":
        controller.handleEscape(event.nativeEvent.isComposing);
        return;
      case "focus-search":
        if (intent.select) controller.enterTextEditMode();
        focusSearch(intent.select);
        return;
      case "switch-panel": {
        if (windowRole === "favorites") return;
        const nextView =
          intent.direction === 1
            ? state.view === "history"
              ? "favorites"
              : "history"
            : state.view === "favorites"
              ? "history"
              : "favorites";
        controller.setView(nextView);
        focusSearch();
        return;
      }
      case "move-selection":
        controller.moveSelection(intent.key);
        if (/^[1-9]$/.test(intent.key)) {
          navigation.activateIndex(Number(intent.key) - 1);
        } else {
          navigation.move(intent.direction);
        }
        return;
      case "toggle-batch-selection": {
        const item = state.items[state.selection];
        if (item) controller.toggleBatchSelection(item.id);
        return;
      }
      case "select-all-batch":
        controller.selectAllBatch();
        return;
      case "primary-action":
        if (controller.selectedItem) {
          if (runtimeContext === "quick-insert") {
            void controller.execute(controller.selectedItem, "insert");
          } else {
            controller.confirmSelection();
          }
        }
        return;
      case "prevent-default":
        return;
    }
  };

  const onFocusCapture = (event: FocusEvent<HTMLElement>) => {
    if (event.target !== searchRef.current) {
      controller.setSearchMode("navigation");
    }
  };

  const selectView = (view: QuickInsertView) => {
    controller.setView(view);
    focusSearch();
  };
  const startTopbarDrag = (event: React.MouseEvent<HTMLDivElement>) => {
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
    try {
      void getCurrentWindow()
        .startDragging()
        .catch(() => undefined);
    } catch {
      // Browser-owned tests do not have a native drag surface.
    }
  };

  const sharedProps = resultProps(
    controller,
    navigation,
    workspaceRef,
    runtimeContext,
  );

  return (
    <main
      className="clipboard-window"
      tabIndex={0}
      onKeyDown={onKeyDown}
      onFocusCapture={onFocusCapture}
      data-testid="clipboard-panel"
      data-window-role={windowRole}
      data-runtime-context={runtimeContext}
      data-search-mode={state.searchMode}
    >
      <div className="clipboard-topbar" onMouseDown={startTopbarDrag}>
        <SearchField
          ref={searchRef}
          className="clipboard-search-row"
          value={state.query}
          onChange={(event) => controller.setQuery(event.target.value)}
          onClick={controller.enterTextEditMode}
          placeholder="Search clipboard history..."
          aria-label="Search clipboard history"
          autoFocus
          autoComplete="off"
          startSlot={
            <EchoIcon
              name="search"
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
            className="clipboard-topbar-action echo-icon-button"
            type="button"
            aria-label="Open settings"
            title="Settings"
            onClick={onOpenSettings}
          >
            <EchoIcon name="settings" size={17} aria-hidden="true" />
          </button>
        ) : null}
      </div>
      {showPanelTabs ? (
        <nav className="clipboard-tabs" aria-label="Clipboard views">
          <div className="clipboard-tab-list" role="tablist">
            {(["history", "favorites"] as const).map((view) => (
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
        </nav>
      ) : null}
      <section
        ref={workspaceRef}
        className="clipboard-entry-workspace"
        aria-label={
          state.view === "favorites" ? "Favorite entries" : "Clipboard entries"
        }
      >
        <div className="clipboard-history-layout">
          {state.view === "favorites" ? (
            <SavedItemsResults
              {...sharedProps}
              updateFavorite={controller.updateFavorite}
              createFavorite={controller.createFavorite}
              reorderFavorites={controller.reorderFavorites}
            />
          ) : (
            <HistoryResults
              {...sharedProps}
              batchMode={state.historyMode === "batch"}
              selectedIds={new Set(state.batchSelectedIds)}
              enterBatchMode={controller.enterBatchMode}
              cancelBatchMode={controller.cancelBatchMode}
              toggleSelected={controller.toggleBatchSelection}
              bulkFavorite={controller.bulkFavorite}
              bulkPin={controller.bulkPin}
              bulkDelete={controller.bulkDelete}
              clearAll={controller.clearUnpinnedHistory}
            />
          )}
        </div>
      </section>
      <footer className="clipboard-footer">
        <div className="key-hints" aria-label="Keyboard controls">
          <span>
            <kbd>Tab</kbd> Panel
          </span>
          <span>
            <kbd>Ctrl + J/K</kbd> Navigate
          </span>
          <span>
            <kbd>Enter</kbd>{" "}
            {runtimeContext === "quick-insert" ? "Paste" : "Select"}
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
  workspaceRef: RefObject<HTMLElement | null>,
  runtimeContext: RuntimeContext,
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
    primaryAction: (
      item: Parameters<typeof controller.execute>[0],
      index: number,
    ) => {
      if (runtimeContext === "quick-insert") {
        void controller.execute(item, "insert");
      } else {
        controller.select(index);
        navigation.activateIndex(index);
      }
    },
    execute: (
      item: Parameters<typeof controller.execute>[0],
      intent?: "insert" | "copy",
    ) => void controller.execute(item, intent),
    toggleFavorite: (item: Parameters<typeof controller.toggleFavorite>[0]) =>
      void controller.toggleFavorite(item),
    remove: (item: Parameters<typeof controller.remove>[0]) =>
      void controller.remove(item),
    isPinned: (item: Parameters<typeof controller.togglePin>[0]) =>
      item.pinned_at !== null,
    togglePin: (item: Parameters<typeof controller.togglePin>[0]) =>
      void controller.togglePin(item),
    hasMore: controller.state.nextCursor !== null,
    loadingMore: controller.state.loadingMore,
    loadMore: () => void controller.loadMore(),
    scrollElementRef: workspaceRef,
  };
}
