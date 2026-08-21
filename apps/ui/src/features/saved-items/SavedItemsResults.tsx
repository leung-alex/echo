import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactElement,
} from "react";

import { FavoriteIcon } from "../../ui/icons/EchoIcon";
import { FavoriteIconPicker } from "../../ui/icons/FavoriteIconPicker";
import type {
  QuickInsertItem,
  SavedItemUpdate,
} from "../quick-insert/model/types";
import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";

export interface FavoriteEditorDraft {
  content: string;
  name: string;
  iconKey: string | null;
  tags: string[];
}

export type CreateFavoriteAdapter = (
  draft: FavoriteEditorDraft,
) => Promise<QuickInsertItem | void> | QuickInsertItem | void;

export interface SavedItemsResultsProps extends SharedEntryResultsProps {
  updateSavedItem: (item: QuickInsertItem, update: SavedItemUpdate) => void;
  createFavorite?: CreateFavoriteAdapter;
  reorderFavorites?: (sourceId: number, targetId: number) => void;
}

type EditorState =
  | { mode: "create"; item: null }
  | { mode: "edit"; item: QuickInsertItem };

export function SavedItemsResults({
  items,
  query,
  selected,
  select,
  updateSavedItem,
  createFavorite,
  reorderFavorites,
  ...props
}: SavedItemsResultsProps): ReactElement {
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [manualOrder, setManualOrder] = useState<string[]>([]);
  const [localFavorites, setLocalFavorites] = useState<QuickInsertItem[]>([]);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const previousEditorRef = useRef<EditorState | null>(null);

  const allItems = useMemo(() => {
    const known = new Set(items.map(itemKey));
    return [
      ...localFavorites.filter((item) => !known.has(itemKey(item))),
      ...items,
    ];
  }, [items, localFavorites]);

  useEffect(() => {
    const incoming = allItems.map(itemKey);
    setManualOrder((current) => {
      const incomingSet = new Set(incoming);
      const retained = current.filter((key) => incomingSet.has(key));
      const retainedSet = new Set(retained);
      const added = incoming.filter((key) => !retainedSet.has(key));
      const next = [...retained, ...added];
      return next.length === current.length &&
        next.every((key, i) => key === current[i])
        ? current
        : next;
    });
  }, [allItems]);

  const orderedItems = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    const matching = normalizedQuery
      ? allItems.filter((item) => matchesQuery(item, normalizedQuery))
      : allItems;
    if (normalizedQuery) return matching;

    const byKey = new Map(matching.map((item) => [itemKey(item), item]));
    return [
      ...manualOrder.flatMap((key) => {
        const item = byKey.get(key);
        return item ? [item] : [];
      }),
      ...matching.filter((item) => !manualOrder.includes(itemKey(item))),
    ];
  }, [allItems, manualOrder, query]);

  const selectedKey =
    selected >= 0 && items[selected] ? itemKey(items[selected]) : null;
  const orderedSelected = selectedKey
    ? orderedItems.findIndex((item) => itemKey(item) === selectedKey)
    : -1;

  useEffect(() => {
    const previousEditor = previousEditorRef.current;
    if (previousEditor && !editor) {
      window.requestAnimationFrame(() => {
        if (previousEditor.mode === "create") {
          addButtonRef.current?.focus();
          return;
        }
        const rowIndex = orderedSelected >= 0 ? orderedSelected : 0;
        document
          .querySelector<HTMLElement>(
            `[data-virtualized-row="true"][data-index="${rowIndex}"]`,
          )
          ?.focus();
      });
    }
    previousEditorRef.current = editor;
  }, [editor, orderedSelected]);

  const selectOrdered = (index: number) => {
    const item = orderedItems[index];
    if (!item) return;
    const sourceIndex = items.findIndex(
      (candidate) => itemKey(candidate) === itemKey(item),
    );
    if (sourceIndex >= 0) select(sourceIndex);
  };

  const handleReorder = (sourceId: number, targetId: number) => {
    setManualOrder((current) =>
      moveOrder(current, `favorite:${sourceId}`, `favorite:${targetId}`),
    );
    reorderFavorites?.(sourceId, targetId);
  };

  const saveEditor = (draft: FavoriteEditorDraft) => {
    if (editor?.mode === "edit") {
      const update: SavedItemUpdate = {
        name: draft.name,
        tags: draft.tags,
        editable_text:
          editor.item.editable_text === null ? null : draft.content,
      };
      // The current generated update contract has no icon field yet. The
      // icon remains presentation-owned until the frozen transport carries it.
      updateSavedItem(editor.item, update);
      setEditor(null);
      return;
    }

    if (editor?.mode === "create") {
      const created = createFavorite?.(draft);
      if (isPromiseLike(created)) {
        void created.then((item) => {
          if (item) setLocalFavorites((current) => [item, ...current]);
        });
      } else if (created) {
        setLocalFavorites((current) => [created, ...current]);
      }
      setEditor(null);
    }
  };

  return (
    <div className="favorites-results" data-testid="favorites-results">
      <header className="favorites-heading">
        <div className="favorites-heading-title">
          <span className="favorites-heading-mark" aria-hidden="true">
            <FavoriteIcon iconKey="StarFilled" size={18} />
          </span>
          <div>
            <span className="history-heading-eyebrow">Durable collection</span>
            <h2>Favorites</h2>
            <span className="history-heading-count">
              {orderedItems.length}{" "}
              {orderedItems.length === 1 ? "item" : "items"}
            </span>
          </div>
        </div>
        <button
          ref={addButtonRef}
          className="favorites-add-button"
          type="button"
          aria-label="Create favorite"
          title="Create favorite"
          onClick={() => setEditor({ mode: "create", item: null })}
        >
          <FavoriteIcon iconKey="Plus" size={18} aria-hidden="true" />
          <span>New</span>
        </button>
      </header>
      <div
        className="favorites-order-note"
        data-searching={Boolean(query.trim())}
      >
        {query.trim()
          ? "Search relevance · drag reorder disabled"
          : "Manual order · drag to rearrange"}
      </div>
      <QuickInsertResults
        {...props}
        items={orderedItems}
        query={query}
        view="favorites"
        selected={orderedSelected}
        select={selectOrdered}
        edit={(item) => setEditor({ mode: "edit", item })}
        reorderEnabled={!query.trim() && !editor}
        onReorder={handleReorder}
      />
      {editor ? (
        <FavoriteEditor
          item={editor.item}
          onCancel={() => setEditor(null)}
          onSave={saveEditor}
        />
      ) : null}
    </div>
  );
}

function FavoriteEditor({
  item,
  onCancel,
  onSave,
}: {
  item: QuickInsertItem | null;
  onCancel: () => void;
  onSave: (draft: FavoriteEditorDraft) => void;
}): ReactElement {
  const iconButtonRef = useRef<HTMLButtonElement>(null);
  const [name, setName] = useState(item?.name ?? "");
  const [iconKey, setIconKey] = useState<string | null>(getIconKey(item));
  const [tags, setTags] = useState(item?.tags.join(", ") ?? "");
  const [content, setContent] = useState(
    item?.editable_text ?? item?.preview_text ?? "",
  );
  const [contentError, setContentError] = useState<string | null>(null);
  const [iconPickerOpen, setIconPickerOpen] = useState(false);
  const contentRef = useRef<HTMLTextAreaElement>(null);
  const editableContent = item?.editable_text !== null;

  const closeIconPicker = () => {
    setIconPickerOpen(false);
    window.requestAnimationFrame(() => iconButtonRef.current?.focus());
  };

  const handleEditorKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (iconPickerOpen) closeIconPicker();
      else onCancel();
      return;
    }
    if (event.key !== "Tab") return;

    event.preventDefault();
    event.stopPropagation();
    const focusable = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((element) => element.tabIndex >= 0);
    if (focusable.length === 0) return;
    const currentIndex = focusable.indexOf(
      document.activeElement as HTMLElement,
    );
    const direction = event.shiftKey ? -1 : 1;
    const nextIndex =
      (currentIndex + direction + focusable.length) % focusable.length;
    focusable[nextIndex]?.focus();
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (editableContent && !content.trim()) {
      setContentError("Content is required.");
      contentRef.current?.focus();
      return;
    }
    setContentError(null);
    onSave({
      content,
      name: name.trim() || content.trim().slice(0, 48) || "Favorite",
      iconKey,
      tags: tags
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
    });
  };

  return (
    <div
      className="echo-confirm-backdrop favorite-editor-backdrop"
      role="presentation"
    >
      <section
        className="favorite-editor"
        role="dialog"
        aria-modal="true"
        aria-label={item ? "Edit favorite" : "Create favorite"}
        onKeyDown={handleEditorKeyDown}
      >
        <div className="favorite-editor__header">
          <div>
            <span className="history-heading-eyebrow">
              {item ? "Edit favorite" : "New favorite"}
            </span>
            <h3>{item ? "Edit Favorite" : "Create Favorite"}</h3>
          </div>
          <button
            className="echo-icon-button"
            type="button"
            aria-label="Close favorite editor"
            title="Close favorite editor"
            onClick={onCancel}
          >
            <FavoriteIcon iconKey="X" size={16} aria-hidden="true" />
          </button>
        </div>
        <form onSubmit={submit}>
          <label>
            Content
            <textarea
              id="favorite-content"
              ref={contentRef}
              value={content}
              onChange={(event) => {
                const nextContent = event.target.value;
                setContent(nextContent);
                if (contentError && nextContent.trim()) {
                  setContentError(null);
                }
              }}
              readOnly={!editableContent}
              aria-required={editableContent}
              aria-invalid={contentError ? "true" : undefined}
              aria-describedby={
                [
                  !editableContent ? "favorite-content-note" : null,
                  contentError ? "favorite-content-error" : null,
                ]
                  .filter(Boolean)
                  .join(" ") || undefined
              }
              aria-errormessage={
                contentError ? "favorite-content-error" : undefined
              }
              rows={5}
              autoFocus
            />
            {contentError ? (
              <p
                id="favorite-content-error"
                className="favorite-field-error"
                role="alert"
              >
                {contentError}
              </p>
            ) : null}
            {!editableContent ? (
              <small id="favorite-content-note">
                Binary content is preserved; edit its metadata below.
              </small>
            ) : null}
          </label>
          <label>
            Name
            <input
              value={name}
              placeholder="Generated from content when blank"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label>
            Icon
            <button
              ref={iconButtonRef}
              className="favorite-icon-field"
              type="button"
              aria-label="Choose favorite icon"
              onClick={() => setIconPickerOpen(true)}
            >
              {iconKey ? <FavoriteIcon iconKey={iconKey} size={17} /> : null}
              <span>
                {iconKey
                  ? iconKey.replace(/([a-z])([A-Z])/g, "$1 $2")
                  : "No icon"}
              </span>
            </button>
          </label>
          <label>
            Tags
            <input
              value={tags}
              placeholder="work, reusable"
              onChange={(event) => setTags(event.target.value)}
            />
            <small>Separate tags with commas.</small>
          </label>
          <div className="favorite-editor__actions">
            <button type="button" onClick={onCancel}>
              Cancel
            </button>
            <button className="primary-action" type="submit">
              Save Favorite
            </button>
          </div>
        </form>
        {iconPickerOpen ? (
          <FavoriteIconPicker
            value={iconKey}
            onChange={(key) => setIconKey(key === "none" ? null : key)}
            onClose={closeIconPicker}
          />
        ) : null}
      </section>
    </div>
  );
}

function getIconKey(item: QuickInsertItem | null): string | null {
  if (!item || !("icon_key" in item)) return null;
  const iconKey = (item as QuickInsertItem & { icon_key?: unknown }).icon_key;
  return typeof iconKey === "string" && iconKey !== "none" ? iconKey : null;
}

function itemKey(item: QuickInsertItem): string {
  return `${item.source}:${item.id}`;
}

function moveOrder(order: string[], source: string, target: string): string[] {
  const sourceIndex = order.indexOf(source);
  const targetIndex = order.indexOf(target);
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) {
    return order;
  }
  const next = [...order];
  const [moved] = next.splice(sourceIndex, 1);
  next.splice(targetIndex, 0, moved);
  return next;
}

function matchesQuery(item: QuickInsertItem, query: string): boolean {
  return `${item.name ?? ""} ${item.preview_text ?? ""} ${item.source_app ?? ""} ${item.tags.join(" ")}`
    .toLocaleLowerCase()
    .includes(query);
}

function isPromiseLike(
  value: QuickInsertItem | void | Promise<QuickInsertItem | void> | undefined,
): value is Promise<QuickInsertItem | void> {
  return (
    typeof value === "object" &&
    value !== null &&
    "then" in value &&
    typeof value.then === "function"
  );
}
