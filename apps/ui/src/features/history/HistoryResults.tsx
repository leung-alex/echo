import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
} from "react";

import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";
import type { QuickInsertItem } from "../quick-insert/model/types";

export interface HistoryResultsProps extends SharedEntryResultsProps {
  clearAll?: () => Promise<void> | void;
}

export function HistoryResults({
  items,
  selected,
  select,
  toggleFavorite,
  remove,
  clearAll,
  ...props
}: HistoryResultsProps): ReactElement {
  const [batchMode, setBatchMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [pinnedIds, setPinnedIds] = useState<Set<number>>(new Set());
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const batchSurfaceRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const available = new Set(items.map((item) => item.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => available.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [items]);

  useEffect(() => {
    if (batchMode) batchSurfaceRef.current?.focus();
  }, [batchMode]);

  const enterBatchMode = () => {
    setClearError(null);
    setBatchMode(true);
    setSelectedIds(new Set());
  };

  const cancelBatchMode = useCallback(() => {
    setBatchMode(false);
    setSelectedIds(new Set());
  }, []);

  const toggleSelected = useCallback((id: number) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const togglePin = useCallback((item: QuickInsertItem) => {
    setPinnedIds((current) => {
      const next = new Set(current);
      if (next.has(item.id)) next.delete(item.id);
      else next.add(item.id);
      return next;
    });
  }, []);

  const selectedItems = items.filter((item) => selectedIds.has(item.id));

  const bulkFavorite = () => {
    selectedItems.forEach((item) => toggleFavorite(item));
    setSelectedIds(new Set());
  };

  const bulkPin = () => {
    setPinnedIds((current) => {
      const next = new Set(current);
      selectedItems.forEach((item) => next.add(item.id));
      return next;
    });
  };

  const bulkDelete = () => {
    selectedItems.forEach((item) => remove(item));
    setSelectedIds(new Set());
  };

  const handleBatchKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!batchMode || event.target !== batchSurfaceRef.current) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cancelBatchMode();
      return;
    }
    if (event.ctrlKey && event.key.toLowerCase() === "a") {
      event.preventDefault();
      event.stopPropagation();
      setSelectedIds(new Set(items.map((item) => item.id)));
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      event.stopPropagation();
      const direction = event.key === "ArrowUp" ? -1 : 1;
      select(Math.max(0, Math.min(items.length - 1, selected + direction)));
      return;
    }
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      const current = items[selected];
      if (current) toggleSelected(current.id);
    }
  };

  const confirmClearAll = async () => {
    if (!clearAll) return;
    setClearError(null);
    try {
      await clearAll();
      setClearConfirmOpen(false);
      cancelBatchMode();
    } catch (error) {
      setClearError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <div
      ref={batchSurfaceRef}
      className={`history-results${batchMode ? " is-batch-mode" : ""}`}
      tabIndex={batchMode ? 0 : -1}
      onKeyDown={handleBatchKeyDown}
      data-testid="history-results"
      aria-label="History presentation"
    >
      <header className="history-heading">
        <div>
          <span className="history-heading-eyebrow">Timeline</span>
          <h2>History</h2>
          <span className="history-heading-count">
            {items.length} {items.length === 1 ? "capture" : "captures"}
          </span>
        </div>
        <div className="history-heading-actions">
          <button
            className="history-heading-button"
            type="button"
            onClick={batchMode ? cancelBatchMode : enterBatchMode}
          >
            {batchMode ? "Cancel" : "Select"}
          </button>
          <button
            className="history-heading-button danger"
            type="button"
            onClick={() => setClearConfirmOpen(true)}
            disabled={!clearAll}
            title={
              clearAll
                ? "Clear unpinned History"
                : "Available after transport wiring"
            }
          >
            Clear all
          </button>
        </div>
      </header>

      {batchMode ? (
        <div
          className="history-batch-toolbar"
          role="toolbar"
          aria-label="History batch actions"
        >
          <span className="history-batch-count">
            {selectedIds.size} selected
          </span>
          <span className="history-batch-hints">
            <kbd>Space</kbd> toggle <kbd>Enter</kbd> select
          </span>
          <div className="history-batch-actions">
            <button
              type="button"
              disabled={selectedIds.size === 0}
              onClick={bulkFavorite}
            >
              Favorite selected
            </button>
            <button
              type="button"
              disabled={selectedIds.size === 0}
              onClick={bulkPin}
            >
              Pin selected
            </button>
            <button
              className="danger"
              type="button"
              disabled={selectedIds.size === 0}
              onClick={bulkDelete}
            >
              Delete selected
            </button>
          </div>
        </div>
      ) : null}

      {clearError ? (
        <p className="history-inline-error" role="alert">
          {clearError}
        </p>
      ) : null}

      <QuickInsertResults
        {...props}
        items={items}
        view="history"
        selected={selected}
        select={select}
        toggleFavorite={toggleFavorite}
        remove={remove}
        selectionMode={batchMode ? "batch" : "browse"}
        selectedIds={selectedIds}
        toggleSelected={toggleSelected}
        isPinned={(item) => pinnedIds.has(item.id)}
        togglePin={togglePin}
      />

      {clearConfirmOpen ? (
        <div className="echo-confirm-backdrop" role="presentation">
          <section
            className="echo-confirm-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="clear-history-title"
          >
            <span className="history-heading-eyebrow">Destructive action</span>
            <h3 id="clear-history-title">Clear unpinned History?</h3>
            <p>
              Pinned items and Favorites are preserved. Only unpinned History
              will be removed.
            </p>
            <div className="echo-confirm-actions">
              <button type="button" onClick={() => setClearConfirmOpen(false)}>
                Cancel
              </button>
              <button
                className="danger"
                type="button"
                onClick={() => void confirmClearAll()}
              >
                Clear unpinned History
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}
