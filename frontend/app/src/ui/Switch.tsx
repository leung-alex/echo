import type { ButtonHTMLAttributes } from "react";

export interface SwitchProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange" | "role"> {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}

export function Switch({ checked, disabled, onCheckedChange, onClick, type = "button", ...props }: SwitchProps) {
  return (
    <button
      {...props}
      aria-checked={checked}
      className={["echo-switch", props.className].filter(Boolean).join(" ")}
      data-state={checked ? "checked" : "unchecked"}
      disabled={disabled}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) onCheckedChange(!checked);
      }}
      role="switch"
      type={type}
    >
      <span aria-hidden="true" className="echo-switch__thumb" />
    </button>
  );
}
