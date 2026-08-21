import {
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent,
  type ReactElement,
} from "react";

import { FavoriteIcon } from "./EchoIcon";
import {
  CURATED_FAVORITE_ICON_KEYS,
  FAVORITE_ICON_KEYS,
  getFavoriteIconComponent,
  NO_ICON_KEY,
  type FavoriteIconKey,
} from "./favorite-icon-catalog";

const INITIAL_VISIBLE_LIMIT = CURATED_FAVORITE_ICON_KEYS.length + 1;
const SEARCH_VISIBLE_LIMIT = 48;

export interface FavoriteIconPickerProps {
  value: string | null | undefined;
  onChange: (value: FavoriteIconKey) => void;
  onClose: () => void;
}

export function FavoriteIconPicker({
  value,
  onChange,
  onClose,
}: FavoriteIconPickerProps): ReactElement {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filteredKeys = useMemo(() => {
    if (!normalizedQuery) return FAVORITE_ICON_KEYS;
    return FAVORITE_ICON_KEYS.filter((key) =>
      formatIconLabel(key).toLocaleLowerCase().includes(normalizedQuery),
    );
  }, [normalizedQuery]);
  const visibleKeys = normalizedQuery
    ? filteredKeys.slice(0, SEARCH_VISIBLE_LIMIT)
    : filteredKeys.slice(0, INITIAL_VISIBLE_LIMIT);

  useEffect(() => {
    setActiveIndex(0);
  }, [normalizedQuery]);

  const choose = (key: FavoriteIconKey) => {
    onChange(key);
    onClose();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (visibleKeys.length === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") {
      event.preventDefault();
      setActiveIndex((current) => (current + 1) % visibleKeys.length);
      return;
    }
    if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
      event.preventDefault();
      setActiveIndex(
        (current) => (current - 1 + visibleKeys.length) % visibleKeys.length,
      );
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const key = visibleKeys[activeIndex];
      if (key) choose(key);
    }
  };

  return (
    <div
      className="favorite-icon-picker"
      role="dialog"
      aria-modal="true"
      aria-label="Choose favorite icon"
      onKeyDown={handleKeyDown}
    >
      <div className="favorite-icon-picker__header">
        <div>
          <span className="history-heading-eyebrow">Apps SDK UI</span>
          <h3>Choose an icon</h3>
        </div>
        <button
          className="echo-icon-button"
          type="button"
          aria-label="Close icon picker"
          title="Close icon picker"
          onClick={onClose}
        >
          <FavoriteIcon iconKey="X" size={16} aria-hidden="true" />
        </button>
      </div>
      <label className="favorite-icon-picker__search">
        <span className="sr-only">Search icons</span>
        <input
          autoFocus
          value={query}
          placeholder="Search the full icon catalog"
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      <div className="favorite-icon-picker__summary" aria-live="polite">
        {normalizedQuery
          ? `${filteredKeys.length} matching icons`
          : "Curated icons · search for the complete catalog"}
      </div>
      <div
        className="favorite-icon-picker__grid"
        role="listbox"
        aria-label="Favorite icon choices"
        tabIndex={0}
      >
        {visibleKeys.map((key, index) => {
          const selected = key === value || (!value && key === NO_ICON_KEY);
          const known =
            key === NO_ICON_KEY || Boolean(getFavoriteIconComponent(key));
          return (
            <button
              className={`favorite-icon-choice${selected ? " is-selected" : ""}${index === activeIndex ? " is-active" : ""}`}
              key={key}
              type="button"
              role="option"
              aria-selected={selected}
              title={formatIconLabel(key)}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => choose(key)}
            >
              <span className="favorite-icon-choice__icon" aria-hidden="true">
                {key === NO_ICON_KEY ? (
                  <span className="favorite-icon-choice__none">No</span>
                ) : known ? (
                  <FavoriteIcon iconKey={key} size={18} />
                ) : null}
              </span>
              <span>{formatIconLabel(key)}</span>
            </button>
          );
        })}
        {visibleKeys.length === 0 ? (
          <p className="favorite-icon-picker__empty">No matching icons.</p>
        ) : null}
      </div>
      <div className="favorite-icon-picker__footer">
        <kbd>↑↓</kbd> navigate <kbd>Enter</kbd> choose <kbd>Esc</kbd> close
      </div>
    </div>
  );
}

function formatIconLabel(key: FavoriteIconKey): string {
  if (key === NO_ICON_KEY) return "No icon";
  return key.replace(/([a-z])([A-Z])/g, "$1 $2");
}
