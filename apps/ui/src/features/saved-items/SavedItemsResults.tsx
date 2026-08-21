import {
  useEffect,
  useMemo,
  useState,
  type FormEvent,
  type ReactElement,
} from "react";

import { EchoIcon } from "../../ui/icons/EchoIcon";
import type {
  QuickInsertItem,
  SavedItemUpdate,
} from "../quick-insert/model/types";
import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";

export interface SavedItemsResultsProps extends SharedEntryResultsProps {
  updateSavedItem: (item: QuickInsertItem, update: SavedItemUpdate) => void;
  deleteSavedItems: (ids: number[]) => Promise<boolean>;
}

export function SavedItemsResults({
  items,
  updateSavedItem,
  deleteSavedItems,
  ...props
}: SavedItemsResultsProps): ReactElement {
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [editing, setEditing] = useState<QuickInsertItem | null>(null);

  useEffect(() => {
    const available = new Set(items.map((item) => item.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => available.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [items]);

  const selected = useMemo(() => [...selectedIds], [selectedIds]);
  const toggleSelected = (id: number) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <>
      {selected.length > 0 ? (
        <div className="saved-items-toolbar">
          <span>{selected.length} selected</span>
          <button
            className="danger"
            type="button"
            onClick={() => {
              void deleteSavedItems(selected).then((deleted) => {
                if (deleted) setSelectedIds(new Set());
              });
            }}
          >
            <EchoIcon name="delete" size={15} aria-hidden="true" />
            Delete selected
          </button>
        </div>
      ) : null}
      <QuickInsertResults
        {...props}
        items={items}
        view="favorites"
        edit={setEditing}
        selectedIds={selectedIds}
        toggleSelected={toggleSelected}
      />
      {editing ? (
        <SavedItemEditor
          item={editing}
          onCancel={() => setEditing(null)}
          onSave={(update) => {
            updateSavedItem(editing, update);
            setEditing(null);
          }}
        />
      ) : null}
    </>
  );
}

function SavedItemEditor({
  item,
  onCancel,
  onSave,
}: {
  item: QuickInsertItem;
  onCancel: () => void;
  onSave: (update: SavedItemUpdate) => void;
}): ReactElement {
  const [name, setName] = useState(item.name ?? "Saved item");
  const [tags, setTags] = useState(item.tags.join(", "));
  const [text, setText] = useState(item.editable_text ?? "");

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    onSave({
      name,
      tags: tags
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
      editable_text: item.editable_text === null ? null : text,
    });
  };

  return (
    <div
      className="saved-item-editor"
      role="dialog"
      aria-label="Edit saved item"
    >
      <form onSubmit={submit}>
        <label>
          Name
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            autoFocus
          />
        </label>
        <label>
          Tags
          <input
            value={tags}
            onChange={(event) => setTags(event.target.value)}
          />
        </label>
        {item.editable_text !== null ? (
          <label>
            Text
            <textarea
              value={text}
              onChange={(event) => setText(event.target.value)}
              rows={6}
            />
          </label>
        ) : null}
        <div className="saved-item-editor-actions">
          <button type="button" onClick={onCancel}>
            <EchoIcon name="close" size={15} aria-hidden="true" />
            Cancel
          </button>
          <button className="primary-action" type="submit">
            <EchoIcon name="check" size={15} aria-hidden="true" />
            Save
          </button>
        </div>
      </form>
    </div>
  );
}
