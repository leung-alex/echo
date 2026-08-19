import { forwardRef, useEffect, useRef, type InputHTMLAttributes, type ReactNode } from "react";

export interface SearchFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "className"> {
  className?: string;
  startSlot?: ReactNode;
  endSlot?: ReactNode;
}

export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  function SearchField({ className, startSlot, endSlot, ...props }, forwardedRef) {
    const inputRef = useRef<HTMLInputElement | null>(null);
    const caretRef = useRef<HTMLSpanElement>(null);

    useEffect(() => {
      const input = inputRef.current;
      const caret = caretRef.current;
      if (!input || !caret) return undefined;
      const sync = () => {
        const visible =
          document.activeElement === input &&
          input.selectionStart === input.selectionEnd;
        caret.dataset.visible = visible ? "true" : "false";
      };
      input.addEventListener("focus", sync);
      input.addEventListener("blur", sync);
      input.addEventListener("select", sync);
      input.addEventListener("input", sync);
      sync();
      return () => {
        input.removeEventListener("focus", sync);
        input.removeEventListener("blur", sync);
        input.removeEventListener("select", sync);
        input.removeEventListener("input", sync);
      };
    }, []);

    return (
      <div className={["echo-search-field", className].filter(Boolean).join(" ")}>
        {startSlot ? <span className="echo-search-field__start">{startSlot}</span> : null}
        <input
          {...props}
          ref={(node) => {
            inputRef.current = node;
            if (typeof forwardedRef === "function") forwardedRef(node);
            else if (forwardedRef) forwardedRef.current = node;
          }}
          className="echo-search-field__input"
          data-custom-caret="false"
        />
        {endSlot ? <span className="echo-search-field__end">{endSlot}</span> : null}
        <span ref={caretRef} className="echo-search-field__caret" data-visible="false" aria-hidden="true" />
      </div>
    );
  },
);
