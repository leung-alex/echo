import { useEffect, useRef, useState, type ReactElement } from "react";

import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";

export interface HistoryResultsProps extends SharedEntryResultsProps {
  batchMode: boolean;
  selectedIds: ReadonlySet<number>;
  enterBatchMode: () => void;
  cancelBatchMode: () => void;
  toggleSelected: (id: number) => void;
  bulkFavorite: (ids: number[]) => Promise<boolean>;
  bulkPin: (ids: number[]) => Promise<boolean>;
  bulkDelete: (ids: number[]) => Promise<boolean>;
  clearAll: () => Promise<void>;
}

export function HistoryResults({
  items,
  selected,
  batchMode,
  selectedIds,
  enterBatchMode,
  cancelBatchMode,
  toggleSelected,
  bulkFavorite,
  bulkPin,
  bulkDelete,
  clearAll,
  ...props
}: HistoryResultsProps): ReactElement {
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const batchSurfaceRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (batchMode) batchSurfaceRef.current?.focus();
  }, [batchMode]);

  const startBatchMode = () => {
    setClearError(null);
    enterBatchMode();
  };

  const confirmClearAll = async () => {
    setClearError(null);
    try {
      await clearAll();
      setClearConfirmOpen(false);
    } catch (error) {
      setClearError(error instanceof Error ? error.message : String(error));
    }
  };

  const selectedIdList = [...selectedIds];

  return (
    <div
      ref={batchSurfaceRef}
      className={`history-results${batchMode ? " is-batch-mode" : ""}`}
      tabIndex={batchMode ? 0 : -1}
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
            onClick={batchMode ? cancelBatchMode : startBatchMode}
          >
            {batchMode ? "Cancel" : "Select"}
          </button>
          <button
            className="history-heading-button danger"
            type="button"
            onClick={() => setClearConfirmOpen(true)}
            title="Clear unpinned History"
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
              onClick={() => void bulkFavorite(selectedIdList)}
            >
              Favorite selected
            </button>
            <button
              type="button"
              disabled={selectedIds.size === 0}
              onClick={() => void bulkPin(selectedIdList)}
            >
              Pin selected
            </button>
            <button
              className="danger"
              type="button"
              disabled={selectedIds.size === 0}
              onClick={() => void bulkDelete(selectedIdList)}
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
        selectionMode={batchMode ? "batch" : "browse"}
        selectedIds={selectedIds}
        toggleSelected={toggleSelected}
        isPinned={(item) => item.pinned_at !== null}
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
