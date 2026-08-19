import {
  forwardRef,
  useCallback,
  useEffect,
  useRef,
  type InputHTMLAttributes,
  type ReactNode,
  type Ref,
} from "react";

export interface CaretVisibilityInput {
  focused: boolean;
  composing: boolean;
  selectionStart: number | null;
  selectionEnd: number | null;
  caretLeft: number;
  inputLeft: number;
  inputRight: number;
}

export function shouldShowCustomCaret({
  focused,
  composing,
  selectionStart,
  selectionEnd,
  caretLeft,
  inputLeft,
  inputRight,
}: CaretVisibilityInput) {
  return (
    focused &&
    !composing &&
    selectionStart !== null &&
    selectionStart === selectionEnd &&
    caretLeft >= inputLeft &&
    caretLeft + 2 <= inputRight
  );
}

export interface SearchFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "className"> {
  className?: string;
  startSlot?: ReactNode;
  endSlot?: ReactNode;
}

function assignRef<T>(ref: Ref<T>, value: T) {
  if (typeof ref === "function") {
    ref(value);
  } else if (ref) {
    ref.current = value;
  }
}

export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  function SearchField(
    {
      className,
      startSlot,
      endSlot,
      value,
      defaultValue,
      onBlur,
      onChange,
      onCompositionEnd,
      onCompositionStart,
      onFocus,
      onKeyUp,
      onScroll,
      onSelect,
      ...inputProps
    },
    forwardedRef,
  ) {
    const rootRef = useRef<HTMLDivElement>(null);
    const inputRef = useRef<HTMLInputElement | null>(null);
    const measureRef = useRef<HTMLSpanElement>(null);
    const caretRef = useRef<HTMLSpanElement>(null);
    const composingRef = useRef(false);
    const animationFrameRef = useRef<number | null>(null);

    const setInputRef = useCallback(
      (input: HTMLInputElement | null) => {
        inputRef.current = input;
        assignRef(forwardedRef, input);
      },
      [forwardedRef],
    );

    const syncCaret = useCallback(() => {
      const root = rootRef.current;
      const input = inputRef.current;
      const measure = measureRef.current;
      const caret = caretRef.current;
      if (!root || !input || !measure || !caret) return;

      const selectionStart = input.selectionStart;
      const selectionEnd = input.selectionEnd;
      const inputStyle = getComputedStyle(input);
      measure.style.font = inputStyle.font;
      measure.style.letterSpacing = inputStyle.letterSpacing;
      const lineHeight = Number.parseFloat(inputStyle.lineHeight);
      if (Number.isFinite(lineHeight)) caret.style.height = `${lineHeight}px`;
      measure.textContent = input.value.slice(0, selectionStart ?? 0);

      const rootRect = root.getBoundingClientRect();
      const inputRect = input.getBoundingClientRect();
      const inputLeft = inputRect.left - rootRect.left;
      const inputRight = inputRect.right - rootRect.left;
      const caretLeft =
        inputLeft + measure.getBoundingClientRect().width - input.scrollLeft;
      const visible = shouldShowCustomCaret({
        focused: document.activeElement === input,
        composing: composingRef.current,
        selectionStart,
        selectionEnd,
        caretLeft,
        inputLeft,
        inputRight,
      });

      input.dataset.customCaret = visible ? "true" : "false";
      caret.dataset.visible = visible ? "true" : "false";
      if (visible) caret.style.transform = `translate(${caretLeft}px, -50%)`;
    }, []);

    const scheduleCaretSync = useCallback(() => {
      if (animationFrameRef.current !== null) {
        cancelAnimationFrame(animationFrameRef.current);
      }
      animationFrameRef.current = requestAnimationFrame(() => {
        animationFrameRef.current = null;
        syncCaret();
      });
    }, [syncCaret]);

    useEffect(() => {
      scheduleCaretSync();
    }, [defaultValue, scheduleCaretSync, value]);

    useEffect(() => {
      const input = inputRef.current;
      const root = rootRef.current;
      if (!input || !root) return;

      let disposed = false;
      const syncLoadedFont = () => {
        if (!disposed) scheduleCaretSync();
      };
      const resizeObserver = new ResizeObserver(scheduleCaretSync);
      resizeObserver.observe(input);
      resizeObserver.observe(root);
      document.addEventListener("selectionchange", scheduleCaretSync);
      void document.fonts.ready.then(syncLoadedFont);

      return () => {
        disposed = true;
        resizeObserver.disconnect();
        document.removeEventListener("selectionchange", scheduleCaretSync);
        if (animationFrameRef.current !== null) {
          cancelAnimationFrame(animationFrameRef.current);
        }
      };
    }, [scheduleCaretSync]);

    return (
      <div
        ref={rootRef}
        className={
          className
            ? `echo-search-field ${className}`
            : "echo-search-field"
        }
      >
        {startSlot ? (
          <span className="echo-search-field__start">{startSlot}</span>
        ) : null}
        <input
          {...inputProps}
          ref={setInputRef}
          className="echo-search-field__input"
          value={value}
          defaultValue={defaultValue}
          onChange={(event) => {
            onChange?.(event);
            scheduleCaretSync();
          }}
          onFocus={(event) => {
            onFocus?.(event);
            scheduleCaretSync();
          }}
          onBlur={(event) => {
            onBlur?.(event);
            scheduleCaretSync();
          }}
          onSelect={(event) => {
            onSelect?.(event);
            scheduleCaretSync();
          }}
          onKeyUp={(event) => {
            onKeyUp?.(event);
            scheduleCaretSync();
          }}
          onScroll={(event) => {
            onScroll?.(event);
            scheduleCaretSync();
          }}
          onCompositionStart={(event) => {
            composingRef.current = true;
            onCompositionStart?.(event);
            scheduleCaretSync();
          }}
          onCompositionEnd={(event) => {
            composingRef.current = false;
            onCompositionEnd?.(event);
            scheduleCaretSync();
          }}
          data-custom-caret="false"
        />
        {endSlot ? (
          <span className="echo-search-field__end">{endSlot}</span>
        ) : null}
        <span
          ref={measureRef}
          className="echo-search-field__measure"
          aria-hidden="true"
        />
        <span
          ref={caretRef}
          className="echo-search-field__caret"
          data-visible="false"
          aria-hidden="true"
        />
      </div>
    );
  },
);
