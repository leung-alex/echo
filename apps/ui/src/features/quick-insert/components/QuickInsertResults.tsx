import {
  useEffect,
  useRef,
  useState,
  type ReactElement,
  type RefObject,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import {
  getSearchMatchIndices,
  SearchMatchText,
} from "../../../ui/SearchMatchText";
import { EchoIcon, FavoriteIcon } from "../../../ui/icons/EchoIcon";
import {
  getFavoriteIconComponent,
  type FavoriteIconKey,
} from "../../../ui/icons/favorite-icon-catalog";
import type { QuickInsertItem, QuickInsertView } from "../model/types";

const VIRTUAL_OVERSCAN = 5;
const DEFAULT_HISTORY_ESTIMATE = 112;
const DEFAULT_FAVORITE_ESTIMATE = 88;

export type ResultSelectionMode = "browse" | "batch";

export interface QuickInsertResultsProps {
  items: QuickInsertItem[];
  query: string;
  view: QuickInsertView;
  selected: number;
  emptyMessage: string;
  getResultId: (key: string) => string;
  select: (index: number) => void;
  execute: (item: QuickInsertItem, intent?: "insert" | "copy") => void;
  toggleFavorite: (item: QuickInsertItem) => void;
  remove: (item: QuickInsertItem) => void;
  edit?: (item: QuickInsertItem) => void;
  selectionMode?: ResultSelectionMode;
  selectedIds?: ReadonlySet<number>;
  toggleSelected?: (id: number) => void;
  isPinned?: (item: QuickInsertItem) => boolean;
  togglePin?: (item: QuickInsertItem) => void;
  reorderEnabled?: boolean;
  onReorder?: (sourceId: number, targetId: number) => void;
  hasMore: boolean;
  loadingMore: boolean;
  loadMore: () => void;
  scrollElementRef: RefObject<HTMLElement | null>;
}

export type SharedEntryResultsProps = Omit<QuickInsertResultsProps, "view">;

export function QuickInsertResults({
  items,
  query,
  view,
  selected,
  emptyMessage,
  getResultId,
  select,
  execute,
  toggleFavorite,
  remove,
  edit,
  selectionMode = "browse",
  selectedIds,
  toggleSelected,
  isPinned,
  togglePin,
  reorderEnabled = false,
  onReorder,
  hasMore,
  loadingMore,
  loadMore,
  scrollElementRef,
}: QuickInsertResultsProps): ReactElement {
  if (items.length === 0) {
    return (
      <EmptyState
        message={
          query.trim() ? `No matches for “${query.trim()}”` : emptyMessage
        }
      />
    );
  }

  return (
    <VirtualizedResults
      items={items}
      query={query}
      view={view}
      selected={selected}
      getResultId={getResultId}
      select={select}
      execute={execute}
      toggleFavorite={toggleFavorite}
      remove={remove}
      edit={edit}
      selectionMode={selectionMode}
      selectedIds={selectedIds}
      toggleSelected={toggleSelected}
      isPinned={isPinned}
      togglePin={togglePin}
      reorderEnabled={reorderEnabled}
      onReorder={onReorder}
      hasMore={hasMore}
      loadingMore={loadingMore}
      loadMore={loadMore}
      scrollElementRef={scrollElementRef}
    />
  );
}

function VirtualizedResults({
  items,
  query,
  view,
  selected,
  getResultId,
  select,
  execute,
  toggleFavorite,
  remove,
  edit,
  selectionMode,
  selectedIds,
  toggleSelected,
  isPinned,
  togglePin,
  reorderEnabled,
  onReorder,
  hasMore,
  loadingMore,
  loadMore,
  scrollElementRef,
}: Omit<QuickInsertResultsProps, "emptyMessage">): ReactElement {
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const justDraggedRef = useRef(false);
  const rowVirtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollElementRef.current,
    estimateSize: () =>
      view === "history" ? DEFAULT_HISTORY_ESTIMATE : DEFAULT_FAVORITE_ESTIMATE,
    measureElement: (element) => element.getBoundingClientRect().height,
    overscan: VIRTUAL_OVERSCAN,
    getItemKey: (index) => {
      const item = items[index];
      return item ? `${item.source}:${item.id}` : index;
    },
  });
  const virtualItems = rowVirtualizer.getVirtualItems();

  useEffect(() => {
    const last = virtualItems[virtualItems.length - 1];
    if (
      hasMore &&
      !loadingMore &&
      last &&
      last.index >= Math.max(0, items.length - VIRTUAL_OVERSCAN - 2)
    ) {
      loadMore();
    }
  }, [hasMore, items.length, loadMore, loadingMore, virtualItems]);

  return (
    <div
      id="echo-entry-results"
      className={
        view === "history" ? "echo-history-list" : "echo-favorites-list"
      }
      role="grid"
      aria-label={
        view === "favorites" ? "Favorite entries" : "Clipboard history"
      }
      style={{ height: rowVirtualizer.getTotalSize() }}
    >
      {virtualItems.map((virtualRow) => {
        const item = items[virtualRow.index];
        if (!item) return null;
        const index = virtualRow.index;
        const selectedByCursor = index === selected;
        const selectedInBatch = selectedIds?.has(item.id) ?? false;
        const pinned = isPinned?.(item) ?? false;
        const favoriteIconKey = getFavoriteIconKey(item);
        const hasFavoriteIcon = Boolean(
          favoriteIconKey && getFavoriteIconComponent(favoriteIconKey),
        );
        const isImage = item.content_type.startsWith("image");
        const actions = (
          <EntryActions
            item={item}
            view={view}
            pinned={pinned}
            copy={() => execute(item, "copy")}
            toggleFavorite={() => toggleFavorite(item)}
            togglePin={togglePin ? () => togglePin(item) : undefined}
            remove={() => remove(item)}
            edit={view === "favorites" ? () => edit?.(item) : undefined}
          />
        );

        return (
          <article
            id={getResultId(`${item.source}:${item.id}`)}
            className={`echo-entry-row echo-entry-row--${view}${selectedByCursor ? " is-cursor" : ""}${selectedInBatch ? " is-batch-selected" : ""}${hasFavoriteIcon ? " has-leading-icon" : ""}${draggingId === item.id ? " is-dragging" : ""}`}
            key={virtualRow.key}
            role="row"
            aria-rowindex={index + 1}
            aria-selected={selectedByCursor || selectedInBatch}
            aria-label={displayText(item)}
            tabIndex={-1}
            data-index={index}
            data-virtualized-row="true"
            draggable={reorderEnabled}
            ref={rowVirtualizer.measureElement}
            style={{
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
              ) {
                return;
              }
              if (!reorderEnabled) event.preventDefault();
              select(index);
            }}
            onClick={() => {
              if (justDraggedRef.current) {
                justDraggedRef.current = false;
                return;
              }
              if (selectionMode === "batch" && toggleSelected) {
                toggleSelected(item.id);
                select(index);
                return;
              }
              execute(item);
            }}
            onDragStart={(event) => {
              if (!reorderEnabled) return;
              justDraggedRef.current = true;
              setDraggingId(item.id);
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", String(item.id));
            }}
            onDragEnd={() => {
              setDraggingId(null);
              window.setTimeout(() => {
                justDraggedRef.current = false;
              }, 0);
            }}
            onDragOver={(event) => {
              if (!reorderEnabled || draggingId === null) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
            }}
            onDrop={(event) => {
              if (!reorderEnabled) return;
              event.preventDefault();
              const sourceId = Number(event.dataTransfer.getData("text/plain"));
              if (Number.isFinite(sourceId) && sourceId !== item.id) {
                onReorder?.(sourceId, item.id);
              }
              setDraggingId(null);
            }}
          >
            {view === "history" ? (
              <HistoryTimeRail timestamp={item.updated_at} />
            ) : hasFavoriteIcon ? (
              <span className="echo-favorite-leading" aria-hidden="true">
                <FavoriteIcon iconKey={favoriteIconKey} size={18} />
              </span>
            ) : null}
            <div className="echo-entry-main" role="gridcell">
              <EntryContent
                item={item}
                query={query}
                imageActions={isImage ? actions : undefined}
              />
              <span className="echo-entry-meta">
                <SearchMatchText
                  text={item.source_app ?? "Unknown source"}
                  indices={getSearchMatchIndices(
                    item.source_app ?? "Unknown source",
                    query,
                  )}
                />
                <span aria-hidden="true">·</span>
                <span>{relativeTime(item.updated_at)}</span>
                {item.tags.length > 0 ? (
                  <span className="echo-entry-tags">
                    {item.tags.map((tag) => (
                      <span className="echo-entry-tag" key={tag}>
                        {tag}
                      </span>
                    ))}
                  </span>
                ) : null}
              </span>
            </div>
            {!isImage ? (
              <span className="echo-history-actions" role="gridcell">
                {actions}
              </span>
            ) : null}
          </article>
        );
      })}
    </div>
  );
}

function HistoryTimeRail({ timestamp }: { timestamp: number }): ReactElement {
  const date = new Date(timestamp);
  return (
    <div className="echo-time-rail" role="gridcell">
      <time dateTime={date.toISOString()}>{formatTime(timestamp)}</time>
      <span className="echo-timeline-dot" aria-hidden="true" />
      <span className="echo-timeline-connector" aria-hidden="true" />
    </div>
  );
}

function EntryContent({
  item,
  query,
  imageActions,
}: {
  item: QuickInsertItem;
  query: string;
  imageActions?: ReactElement;
}): ReactElement {
  const isImage = item.content_type.startsWith("image");
  const preview = displayText(item);
  return (
    <div className="echo-content-preview" title={preview}>
      {isImage ? (
        <div className="echo-image-preview-shell">
          {item.preview ? <PreviewImage item={item} /> : <ImagePlaceholder />}
          {imageActions ? (
            <span className="echo-image-actions" role="gridcell">
              {imageActions}
            </span>
          ) : null}
        </div>
      ) : (
        <SearchMatchText
          text={preview}
          indices={getSearchMatchIndices(preview, query)}
        />
      )}
    </div>
  );
}

function PreviewImage({ item }: { item: QuickInsertItem }): ReactElement {
  const [failed, setFailed] = useState(false);
  const preview = item.preview;
  if (!preview || failed) return <ImagePlaceholder />;

  return (
    <figure
      className="echo-inline-image"
      style={{
        aspectRatio: `${Math.max(1, preview.width)} / ${Math.max(1, preview.height)}`,
      }}
    >
      <img
        src={preview.url}
        alt="Inline clipboard image preview"
        loading="lazy"
        width={preview.width}
        height={preview.height}
        onError={() => setFailed(true)}
      />
    </figure>
  );
}

function ImagePlaceholder(): ReactElement {
  return (
    <span
      className="echo-image-placeholder"
      role="img"
      aria-label="Image preview unavailable"
    >
      Image preview unavailable
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

function getFavoriteIconKey(
  item: QuickInsertItem,
): FavoriteIconKey | string | null {
  // `icon_key` is an optional presentation adapter field until the frozen
  // Favorites transport exposes it. It is never written to generated DTOs.
  if (!("icon_key" in item)) return null;
  const iconKey = (item as QuickInsertItem & { icon_key?: unknown }).icon_key;
  return typeof iconKey === "string" ? iconKey : null;
}

function EntryActions({
  item,
  view,
  pinned,
  copy,
  toggleFavorite,
  togglePin,
  remove,
  edit,
}: {
  item: QuickInsertItem;
  view: QuickInsertView;
  pinned: boolean;
  copy: () => void;
  toggleFavorite: () => void;
  togglePin?: () => void;
  remove: () => void;
  edit?: () => void;
}): ReactElement {
  if (view === "favorites") {
    return (
      <div className="echo-row-actions" aria-label="Favorite actions">
        <ActionButton
          label="Copy"
          icon="copy"
          onClick={copy}
          testId="favorite-copy-action"
        />
        <ActionButton
          label="Edit saved item"
          icon="edit"
          onClick={() => edit?.()}
          testId="favorite-edit-action"
        />
        <ActionButton
          label="Delete"
          icon="delete"
          onClick={remove}
          danger
          testId="favorite-delete-action"
        />
      </div>
    );
  }

  return (
    <div className="echo-row-actions" aria-label="History actions">
      <ActionButton
        label="Favorite"
        icon="favorite"
        onClick={toggleFavorite}
        testId="history-favorite-action"
      />
      <ActionButton
        label={pinned ? "Unpin" : "Pin"}
        icon={pinned ? "unpin" : "pin"}
        onClick={togglePin}
        pressed={pinned}
        testId="history-pin-action"
      />
      <ActionButton
        label="Copy"
        icon="copy"
        onClick={copy}
        testId="history-copy-action"
      />
      <ActionButton
        label="Delete"
        icon="delete"
        onClick={remove}
        danger
        testId="history-delete-action"
      />
    </div>
  );
}

function ActionButton({
  label,
  icon,
  onClick,
  pressed,
  danger = false,
  testId,
}: {
  label: string;
  icon: Parameters<typeof EchoIcon>[0]["name"];
  onClick?: () => void;
  pressed?: boolean;
  danger?: boolean;
  testId: string;
}): ReactElement {
  return (
    <button
      className={`echo-direct-action${danger ? " danger" : ""}`}
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      data-testid={testId}
      onPointerDown={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClick?.();
      }}
    >
      <EchoIcon name={icon} size={16} aria-hidden="true" />
    </button>
  );
}

function EmptyState({ message }: { message: string }): ReactElement {
  return (
    <div className="echo-empty-state" role="status" data-empty-state="true">
      <span className="echo-empty-mark" aria-hidden="true">
        <EchoIcon name="history" size={22} />
      </span>
      <strong>{message}</strong>
      <small>Keep Echo running to capture new content.</small>
    </div>
  );
}

function formatTime(timestamp: number): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(timestamp));
}

function relativeTime(timestamp: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1000));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
