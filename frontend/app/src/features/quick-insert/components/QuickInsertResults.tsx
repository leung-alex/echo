import { useEffect, useState, type ReactElement } from "react";
import { Copy, Star, Trash2 } from "lucide-react";

import { getSearchMatchIndices, SearchMatchText } from "../../../ui/SearchMatchText";
import type { QuickInsertItem, QuickInsertView } from "../model/types";

const imagePreviewCache = new Map<string, string | null>();

interface QuickInsertResultsProps {
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
  getImage: (source: QuickInsertItem["source"], id: number) => Promise<string | null>;
}

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
  getImage,
}: QuickInsertResultsProps): ReactElement {
  if (items.length === 0) return <EmptyState message={emptyMessage} />;

  if (view === "snippets") {
    return (
      <div id="echo-snippet-results" className="echo-result-list" role="listbox" aria-label="Snippet results">
        {items.map((item, index) => (
          <SnippetRow
            key={`${item.source}:${item.id}`}
            item={item}
            id={getResultId(`${item.source}:${item.id}`)}
            index={index}
            selected={index === selected}
            query={query}
            select={() => select(index)}
            execute={() => execute(item)}
            copy={() => execute(item, "copy")}
            remove={() => remove(item)}
          />
        ))}
      </div>
    );
  }

  const className = viewMode === "compact" ? "echo-compact-list" : "echo-history-list";
  return (
    <div
      id="echo-entry-results"
      className={className}
      role="grid"
      aria-label={view === "favorites" ? "Favorite entries" : "Clipboard history"}
    >
      {items.map((item, index) => (
        <div
          id={getResultId(`${item.source}:${item.id}`)}
          className={`echo-history-row${viewMode === "compact" ? " echo-history-row--compact" : ""}`}
          key={`${item.source}:${item.id}`}
          role="row"
          aria-selected={index === selected}
          tabIndex={-1}
          onPointerDown={(event) => {
            if (event.target instanceof Element && event.target.closest("button, input, textarea, select, a")) return;
            event.preventDefault();
            select(index);
          }}
          onClick={() => execute(item)}
        >
          <span className="echo-type-mark" aria-hidden="true">
            {item.content_type.startsWith("image") ? "IMG" : item.content_type.slice(0, 3).toUpperCase()}
          </span>
          <span className="echo-history-copy" role="gridcell">
            <EntryContent item={item} query={query} getImage={getImage} />
            <span className="echo-history-meta">
              <SearchMatchText
                text={item.source_app ?? "Unknown source"}
                indices={getSearchMatchIndices(item.source_app ?? "Unknown source", query)}
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
            />
          </span>
        </div>
      ))}
    </div>
  );
}

function EntryContent({
  item,
  query,
  getImage,
}: {
  item: QuickInsertItem;
  query: string;
  getImage: QuickInsertResultsProps["getImage"];
}) {
  const cacheKey = `${item.source}:${item.id}`;
  const [imageUrl, setImageUrl] = useState<string | null>(() => imagePreviewCache.get(cacheKey) ?? null);
  const isImage = item.content_type.startsWith("image");

  useEffect(() => {
    if (!isImage || imagePreviewCache.has(cacheKey)) return undefined;
    let active = true;
    void getImage(item.source, item.id)
      .then((url) => {
        imagePreviewCache.set(cacheKey, url);
        if (active) setImageUrl(url);
      })
      .catch(() => imagePreviewCache.set(cacheKey, null));
    return () => {
      active = false;
    };
  }, [cacheKey, getImage, isImage, item.id, item.source]);

  const preview = item.preview_text || item.title || "Empty content";
  return (
    <span className="echo-content-preview" title={preview}>
      {imageUrl ? <img src={imageUrl} alt="Clipboard image preview" loading="lazy" /> : <SearchMatchText text={preview} indices={getSearchMatchIndices(preview, query)} />}
    </span>
  );
}

function EntryActions({
  item,
  copy,
  toggleFavorite,
  remove,
}: {
  item: QuickInsertItem;
  copy: () => void;
  toggleFavorite: () => void;
  remove: () => void;
}) {
  const pinned = item.source === "favorite" || item.pinned;
  return (
    <div className="echo-row-actions">
      <button type="button" aria-label="Copy" title="Copy" onPointerDown={(event) => event.preventDefault()} onClick={(event) => { event.stopPropagation(); copy(); }}>
        <Copy size={16} aria-hidden="true" />
      </button>
      <button type="button" aria-label={pinned ? "Unfavorite" : "Favorite"} aria-pressed={pinned} title={pinned ? "Unfavorite" : "Favorite"} onPointerDown={(event) => event.preventDefault()} onClick={(event) => { event.stopPropagation(); toggleFavorite(); }}>
        <Star size={17} fill={pinned ? "currentColor" : "none"} aria-hidden="true" />
      </button>
      <button className="danger" type="button" aria-label="Delete" title="Delete" onPointerDown={(event) => event.preventDefault()} onClick={(event) => { event.stopPropagation(); remove(); }}>
        <Trash2 size={16} aria-hidden="true" />
      </button>
    </div>
  );
}

function SnippetRow({
  item,
  id,
  index,
  selected,
  query,
  select,
  execute,
  copy,
  remove,
}: {
  item: QuickInsertItem;
  id: string;
  index: number;
  selected: boolean;
  query: string;
  select: () => void;
  execute: () => void;
  copy: () => void;
  remove: () => void;
}) {
  const title = item.title || "Untitled snippet";
  const preview = item.preview_text || "Empty snippet";
  return (
    <div id={id} className="echo-snippet-row" role="option" aria-selected={selected} tabIndex={-1} onPointerDown={(event) => { if (event.target instanceof Element && event.target.closest("button, input, textarea, select, a")) return; event.preventDefault(); select(); }} onClick={execute}>
      <span className="echo-snippet-icon" aria-hidden="true">S</span>
      <span className="echo-snippet-copy">
        <strong><SearchMatchText text={title} indices={getSearchMatchIndices(title, query)} /></strong>
        <span><SearchMatchText text={preview} indices={getSearchMatchIndices(preview, query)} /></span>
        <small>{item.group_name || "Ungrouped"} · {relativeTime(item.updated_at)}</small>
      </span>
      <span className="echo-row-actions">
        <button type="button" aria-label="Copy" title="Copy" onPointerDown={(event) => event.preventDefault()} onClick={(event) => { event.stopPropagation(); copy(); }}><Copy size={16} aria-hidden="true" /></button>
        <button className="danger" type="button" aria-label="Delete" title="Delete" onPointerDown={(event) => event.preventDefault()} onClick={(event) => { event.stopPropagation(); remove(); }}><Trash2 size={16} aria-hidden="true" /></button>
      </span>
      <span className="sr-only">Result {index + 1}</span>
    </div>
  );
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="echo-empty-state">
      <span className="echo-empty-mark" aria-hidden="true">E</span>
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
