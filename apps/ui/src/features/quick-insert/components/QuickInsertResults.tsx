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
// These are only first-pass estimates. Every mounted row is measured so text
// wrapping, image metadata, action states, and viewport changes can settle on
// their actual height.
const DEFAULT_HISTORY_ESTIMATE = 112;
const DEFAULT_FAVORITE_ESTIMATE = 88;
const POINTER_REORDER_THRESHOLD = 4;

export type ResultSelectionMode = "browse" | "batch";

export interface QuickInsertResultsProps {
  items: QuickInsertItem[];
  query: string;
  view: QuickInsertView;
  selected: number;
  emptyMessage: string;
  getResultId: (key: string) => string;
  select: (index: number) => void;
  primaryAction: (item: QuickInsertItem, index: number) => void;
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
  primaryAction,
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
      primaryAction={primaryAction}
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
  primaryAction,
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
  const pointerReorderRef = useRef<{
    pointerId: number;
    sourceId: number;
    targetId: number | null;
    startX: number;
    startY: number;
    active: boolean;
  } | null>(null);
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
    const scrollElement = scrollElementRef.current;
    if (!scrollElement || typeof ResizeObserver === "undefined") return;

    let lastWidth = scrollElement.getBoundingClientRect().width;
    const resizeObserver = new ResizeObserver(([entry]) => {
      const nextWidth = entry?.contentRect.width;
      if (nextWidth === undefined || nextWidth === lastWidth) return;
      lastWidth = nextWidth;
      rowVirtualizer.measure();
    });
    resizeObserver.observe(scrollElement);
    return () => resizeObserver.disconnect();
  }, [rowVirtualizer, scrollElementRef]);

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
      aria-rowcount={items.length}
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
        const keyboardActionsVisible = selectedByCursor || selectedInBatch;
        const rowCanReceiveTab =
          selectedByCursor || (selected < 0 && index === 0);
        const actions = (
          <EntryActions
            item={item}
            view={view}
            pinned={pinned}
            keyboardReachable={keyboardActionsVisible}
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
            tabIndex={rowCanReceiveTab ? 0 : -1}
            data-index={index}
            data-item-id={item.id}
            data-virtualized-row="true"
            draggable={false}
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
              if (event.button !== 0) return;
              if (!reorderEnabled) event.preventDefault();
              if (reorderEnabled) {
                pointerReorderRef.current = {
                  pointerId: event.pointerId,
                  sourceId: item.id,
                  targetId: item.id,
                  startX: event.clientX,
                  startY: event.clientY,
                  active: false,
                };
                event.currentTarget.setPointerCapture(event.pointerId);
              }
              select(index);
            }}
            onPointerMove={(event) => {
              const pointerReorder = pointerReorderRef.current;
              if (
                !pointerReorder ||
                pointerReorder.pointerId !== event.pointerId
              ) {
                return;
              }
              if (!pointerReorder.active) {
                const distance = Math.hypot(
                  event.clientX - pointerReorder.startX,
                  event.clientY - pointerReorder.startY,
                );
                if (distance < POINTER_REORDER_THRESHOLD) return;
                pointerReorder.active = true;
                justDraggedRef.current = true;
                setDraggingId(pointerReorder.sourceId);
              }
              event.preventDefault();
              const target = document
                .elementFromPoint(event.clientX, event.clientY)
                ?.closest<HTMLElement>(
                  '[data-virtualized-row="true"][data-item-id]',
                );
              const targetId = Number(target?.dataset.itemId);
              if (
                Number.isSafeInteger(targetId) &&
                items.some((candidate) => candidate.id === targetId)
              ) {
                pointerReorder.targetId = targetId;
              } else {
                pointerReorder.targetId = null;
              }
            }}
            onPointerUp={(event) => {
              const pointerReorder = pointerReorderRef.current;
              if (
                !pointerReorder ||
                pointerReorder.pointerId !== event.pointerId
              ) {
                return;
              }
              pointerReorderRef.current = null;
              if (pointerReorder.active) {
                event.preventDefault();
                const targetId = pointerReorder.targetId;
                if (targetId !== null && pointerReorder.sourceId !== targetId) {
                  onReorder?.(pointerReorder.sourceId, targetId);
                }
                setDraggingId(null);
                window.setTimeout(() => {
                  justDraggedRef.current = false;
                }, 0);
              }
              if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                event.currentTarget.releasePointerCapture(event.pointerId);
              }
            }}
            onPointerCancel={(event) => {
              const pointerReorder = pointerReorderRef.current;
              if (
                !pointerReorder ||
                pointerReorder.pointerId !== event.pointerId
              ) {
                return;
              }
              pointerReorderRef.current = null;
              setDraggingId(null);
              justDraggedRef.current = false;
              if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                event.currentTarget.releasePointerCapture(event.pointerId);
              }
            }}
            onFocus={() => {
              if (selected !== index) select(index);
            }}
            onKeyDown={(event) => {
              if (
                event.target instanceof Element &&
                event.target.closest("button, input, textarea, select, a")
              ) {
                return;
              }
              if (
                selectionMode === "batch" &&
                toggleSelected &&
                (event.key === " " || event.key === "Enter")
              ) {
                event.preventDefault();
                event.stopPropagation();
                toggleSelected(item.id);
                if (selected !== index) select(index);
                return;
              }
              if (selectionMode === "browse" && event.key === "Enter") {
                event.preventDefault();
                event.stopPropagation();
                primaryAction(item, index);
              }
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
              primaryAction(item, index);
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
            <span className="echo-image-actions">{imageActions}</span>
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
  return item.icon_key;
}

function EntryActions({
  item,
  view,
  pinned,
  keyboardReachable,
  copy,
  toggleFavorite,
  togglePin,
  remove,
  edit,
}: {
  item: QuickInsertItem;
  view: QuickInsertView;
  pinned: boolean;
  keyboardReachable: boolean;
  copy: () => void;
  toggleFavorite: () => void;
  togglePin?: () => void;
  remove: () => void;
  edit?: () => void;
}): ReactElement {
  if (view === "favorites") {
    return (
      <div
        className="echo-row-actions"
        role="group"
        aria-label="Favorite actions"
      >
        <ActionButton
          label="Copy"
          icon="copy"
          tabIndex={keyboardReachable ? 0 : -1}
          onClick={copy}
          testId="favorite-copy-action"
        />
        <ActionButton
          label="Edit"
          icon="edit"
          tabIndex={keyboardReachable ? 0 : -1}
          onClick={() => edit?.()}
          testId="favorite-edit-action"
        />
        <ActionButton
          label="Delete"
          icon="delete"
          tabIndex={keyboardReachable ? 0 : -1}
          onClick={remove}
          danger
          testId="favorite-delete-action"
        />
      </div>
    );
  }

  return (
    <div className="echo-row-actions" role="group" aria-label="History actions">
      <ActionButton
        label="Favorite"
        icon="favorite"
        tabIndex={keyboardReachable ? 0 : -1}
        onClick={toggleFavorite}
        testId="history-favorite-action"
      />
      <ActionButton
        label={pinned ? "Unpin" : "Pin"}
        icon={pinned ? "unpin" : "pin"}
        tabIndex={keyboardReachable ? 0 : -1}
        onClick={togglePin}
        pressed={pinned}
        testId="history-pin-action"
      />
      <ActionButton
        label="Copy"
        icon="copy"
        tabIndex={keyboardReachable ? 0 : -1}
        onClick={copy}
        testId="history-copy-action"
      />
      <ActionButton
        label="Delete"
        icon="delete"
        tabIndex={keyboardReachable ? 0 : -1}
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
  tabIndex,
  onClick,
  pressed,
  danger = false,
  testId,
}: {
  label: string;
  icon: Parameters<typeof EchoIcon>[0]["name"];
  tabIndex: number;
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
      tabIndex={tabIndex}
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
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
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
