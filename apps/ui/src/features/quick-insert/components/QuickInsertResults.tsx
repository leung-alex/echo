import { useState, type ReactElement, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Copy, Pencil, Star, Trash2 } from "lucide-react";

import {
  getSearchMatchIndices,
  SearchMatchText,
} from "../../../ui/SearchMatchText";
import type { QuickInsertItem, QuickInsertView } from "../model/types";

const VIRTUAL_OVERSCAN = 5;

export interface QuickInsertResultsProps {
  items: QuickInsertItem[];
  query: string;
  view: QuickInsertView;
  viewMode: "detailed" | "compact";
  selected: number;
  emptyMessage: string;
  getResultId: (key: string) => string;
  select: (index: number) => void;
  execute: (item: QuickInsertItem, intent?: "insert" | "copy") => void;
  toggleFavorite: (item: QuickInsertItem) => void;
  remove: (item: QuickInsertItem) => void;
  edit?: (item: QuickInsertItem) => void;
  selectedIds?: Set<number>;
  toggleSelected?: (id: number) => void;
  scrollElementRef: RefObject<HTMLElement | null>;
}

export type SharedEntryResultsProps = Omit<QuickInsertResultsProps, "view">;

export function QuickInsertResults({
  items,
  query,
  view,
  viewMode,
  selected,
  emptyMessage,
  getResultId,
  select,
  execute,
  toggleFavorite,
  remove,
  edit,
  selectedIds,
  toggleSelected,
  scrollElementRef,
}: QuickInsertResultsProps): ReactElement {
  if (items.length === 0) return <EmptyState message={emptyMessage} />;

  return (
    <VirtualizedResults
      items={items}
      query={query}
      view={view}
      viewMode={viewMode}
      selected={selected}
      emptyMessage={emptyMessage}
      getResultId={getResultId}
      select={select}
      execute={execute}
      toggleFavorite={toggleFavorite}
      remove={remove}
      edit={edit}
      selectedIds={selectedIds}
      toggleSelected={toggleSelected}
      scrollElementRef={scrollElementRef}
    />
  );
}

function VirtualizedResults({
  items,
  query,
  view,
  viewMode,
  selected,
  getResultId,
  select,
  execute,
  toggleFavorite,
  remove,
  edit,
  selectedIds,
  toggleSelected,
  scrollElementRef,
}: QuickInsertResultsProps): ReactElement {
  const rowHeight = viewMode === "compact" ? 52 : 76;
  const rowVirtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollElementRef.current,
    estimateSize: () => rowHeight + 6,
    overscan: VIRTUAL_OVERSCAN,
    getItemKey: (index) => {
      const item = items[index];
      return item ? `${item.source}:${item.id}` : index;
    },
  });

  const className =
    viewMode === "compact" ? "echo-compact-list" : "echo-history-list";
  return (
    <div
      id="echo-entry-results"
      className={className}
      role="grid"
      aria-label={
        view === "favorites" ? "Favorite entries" : "Clipboard history"
      }
      style={{ height: rowVirtualizer.getTotalSize() }}
    >
      {rowVirtualizer.getVirtualItems().map((virtualRow) => {
        const item = items[virtualRow.index];
        if (!item) return null;
        const index = virtualRow.index;
        return (
          <div
            id={getResultId(`${item.source}:${item.id}`)}
            className={`echo-history-row${viewMode === "compact" ? " echo-history-row--compact" : ""}${view === "favorites" && selectedIds ? " echo-saved-row" : ""}`}
            key={virtualRow.key}
            role="row"
            aria-rowindex={index + 1}
            aria-selected={index === selected}
            aria-label={displayText(item)}
            tabIndex={-1}
            data-virtualized-row="true"
            style={{
              height: rowHeight,
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualRow.start}px)`,
            }}
            onPointerDown={(event) => {
              if (
                event.target instanceof Element &&
                event.target.closest("button, input, textarea, select, a")
              )
                return;
              event.preventDefault();
              select(index);
            }}
            onClick={() => execute(item)}
          >
            {view === "favorites" && selectedIds && toggleSelected ? (
              <input
                type="checkbox"
                aria-label={`Select ${item.name ?? "saved item"}`}
                checked={selectedIds.has(item.id)}
                onPointerDown={(event) => event.stopPropagation()}
                onChange={() => toggleSelected(item.id)}
                onClick={(event) => event.stopPropagation()}
              />
            ) : null}
            <span className="echo-type-mark" aria-hidden="true">
              {item.content_type.startsWith("image")
                ? "IMG"
                : item.content_type.slice(0, 3).toUpperCase()}
            </span>
            <span className="echo-history-copy" role="gridcell">
              <EntryContent item={item} query={query} />
              <span className="echo-history-meta">
                <SearchMatchText
                  text={item.source_app ?? "Unknown source"}
                  indices={getSearchMatchIndices(
                    item.source_app ?? "Unknown source",
                    query,
                  )}
                />
                <span aria-hidden="true">·</span>
                <span>{relativeTime(item.updated_at)}</span>
              </span>
            </span>
            <span className="echo-history-actions" role="gridcell">
              <EntryActions
                item={item}
                copy={() => execute(item, "copy")}
                toggleFavorite={() => toggleFavorite(item)}
                remove={() => remove(item)}
                edit={view === "favorites" ? () => edit?.(item) : undefined}
              />
            </span>
          </div>
        );
      })}
    </div>
  );
}

function EntryContent({
  item,
  query,
}: {
  item: QuickInsertItem;
  query: string;
}) {
  const isImage = item.content_type.startsWith("image");
  const preview = displayText(item);
  return (
    <span className="echo-content-preview" title={preview}>
      {isImage ? (
        item.preview ? (
          <PreviewImage url={item.preview.url} />
        ) : (
          <ImagePlaceholder />
        )
      ) : (
        <SearchMatchText
          text={preview}
          indices={getSearchMatchIndices(preview, query)}
        />
      )}
    </span>
  );
}

function PreviewImage({ url }: { url: string }): ReactElement {
  const [failed, setFailed] = useState(false);
  if (failed) return <ImagePlaceholder />;
  return (
    <img
      src={url}
      alt="Clipboard image preview"
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

function ImagePlaceholder(): ReactElement {
  return (
    <span
      className="echo-image-placeholder"
      role="img"
      aria-label="Image preview unavailable"
    >
      IMG
    </span>
  );
}

function displayText(item: QuickInsertItem): string {
  return (
    (item.source === "favorite"
      ? item.name || item.preview_text
      : item.preview_text || item.name) ?? "Empty content"
  );
}

function EntryActions({
  item,
  copy,
  toggleFavorite,
  remove,
  edit,
}: {
  item: QuickInsertItem;
  copy: () => void;
  toggleFavorite: () => void;
  remove: () => void;
  edit?: () => void;
}) {
  const saved = item.source === "favorite" || item.saved_item_id !== null;
  return (
    <div className="echo-row-actions">
      <button
        type="button"
        aria-label="Copy"
        title="Copy"
        onPointerDown={(event) => event.preventDefault()}
        onClick={(event) => {
          event.stopPropagation();
          copy();
        }}
      >
        <Copy size={16} aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label={saved ? "Unfavorite" : "Favorite"}
        aria-pressed={saved}
        title={saved ? "Unfavorite" : "Favorite"}
        onPointerDown={(event) => event.preventDefault()}
        onClick={(event) => {
          event.stopPropagation();
          toggleFavorite();
        }}
      >
        <Star
          size={17}
          fill={saved ? "currentColor" : "none"}
          aria-hidden="true"
        />
      </button>
      {edit ? (
        <button
          type="button"
          aria-label="Edit saved item"
          title="Edit saved item"
          onPointerDown={(event) => event.preventDefault()}
          onClick={(event) => {
            event.stopPropagation();
            edit();
          }}
        >
          <Pencil size={16} aria-hidden="true" />
        </button>
      ) : null}
      <button
        className="danger"
        type="button"
        aria-label="Delete"
        title="Delete"
        onPointerDown={(event) => event.preventDefault()}
        onClick={(event) => {
          event.stopPropagation();
          remove();
        }}
      >
        <Trash2 size={16} aria-hidden="true" />
      </button>
    </div>
  );
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="echo-empty-state">
      <span className="echo-empty-mark" aria-hidden="true">
        E
      </span>
      <strong>{message}</strong>
      <small>Keep Echo running to capture new content.</small>
    </div>
  );
}

function relativeTime(timestamp: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1000));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
