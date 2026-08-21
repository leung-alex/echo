import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactElement,
} from "react";

import { FavoriteIcon } from "../../ui/icons/EchoIcon";
import { FavoriteIconPicker } from "../../ui/icons/FavoriteIconPicker";
import type {
  FavoriteDraft,
  FavoriteUpdate,
  QuickInsertItem,
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

export interface SavedItemsResultsProps extends SharedEntryResultsProps {
  updateFavorite: (
    item: QuickInsertItem,
    update: FavoriteUpdate,
  ) => Promise<void>;
  createFavorite: (draft: FavoriteDraft) => Promise<QuickInsertItem>;
  reorderFavorites: (orderedIds: number[]) => Promise<void>;
}

type EditorState =
  | { mode: "create"; item: null }
  | { mode: "edit"; item: QuickInsertItem };

export function SavedItemsResults({
  items,
  query,
  selected,
  select,
  updateFavorite,
  createFavorite,
  reorderFavorites,
  ...props
}: SavedItemsResultsProps): ReactElement {
  const [editor, setEditor] = useState<EditorState | null>(null);
  const addButtonRef = useRef<HTMLButtonElement>(null);
  const previousEditorRef = useRef<EditorState | null>(null);
  const orderedItems = items;

  useEffect(() => {
    const previousEditor = previousEditorRef.current;
    if (previousEditor && !editor) {
      window.requestAnimationFrame(() => {
        if (previousEditor.mode === "create") {
          addButtonRef.current?.focus();
          return;
        }
        const rowIndex = selected >= 0 ? selected : 0;
        document
          .querySelector<HTMLElement>(
            `[data-virtualized-row="true"][data-index="${rowIndex}"]`,
          )
          ?.focus();
      });
    }
    previousEditorRef.current = editor;
  }, [editor, selected]);

  const handleReorder = (sourceId: number, targetId: number) => {
    const sourceIndex = orderedItems.findIndex((item) => item.id === sourceId);
    const targetIndex = orderedItems.findIndex((item) => item.id === targetId);
    if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex)
      return;
    const next = orderedItems.map((item) => item.id);
    const [moved] = next.splice(sourceIndex, 1);
    next.splice(targetIndex, 0, moved);
    void reorderFavorites(next).catch(() => undefined);
  };

  const saveEditor = async (draft: FavoriteEditorDraft) => {
    if (editor?.mode === "edit") {
      const update: FavoriteUpdate = {
        name: draft.name || null,
        icon_key: draft.iconKey,
        tags: draft.tags,
        editable_text:
          editor.item.editable_text === null ? null : draft.content,
      };
      try {
        await updateFavorite(editor.item, update);
        setEditor(null);
      } catch {
        // The controller reports the transport error and keeps the editor open.
      }
      return;
    }

    if (editor?.mode === "create") {
      try {
        await createFavorite({
          content: draft.content,
          name: draft.name || null,
          icon_key: draft.iconKey,
          tags: draft.tags,
        });
        setEditor(null);
      } catch {
        // The controller reports the transport error and keeps the editor open.
      }
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
        selected={selected}
        select={select}
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
  onSave: (draft: FavoriteEditorDraft) => Promise<void>;
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
  const [saving, setSaving] = useState(false);
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

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (saving) return;
    if (editableContent && !content.trim()) {
      setContentError("Content is required.");
      contentRef.current?.focus();
      return;
    }
    setContentError(null);
    setSaving(true);
    try {
      await onSave({
        content,
        name: name.trim() || content.trim().slice(0, 48) || "Favorite",
        iconKey,
        tags: tags
          .split(",")
          .map((tag) => tag.trim())
          .filter(Boolean),
      });
    } finally {
      setSaving(false);
    }
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
            <button className="primary-action" type="submit" disabled={saving}>
              {saving ? "Saving…" : "Save Favorite"}
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
  const iconKey = item?.icon_key;
  return iconKey && iconKey !== "none" ? iconKey : null;
}
