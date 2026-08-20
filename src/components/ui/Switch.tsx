import React from "react";

/** 开关（勾选 = 加入/开启）。role="switch" + aria-checked，键盘可操作，150ms 过渡。 */
export function Switch({
  checked,
  onChange,
  disabled = false,
  ariaLabel,
  size = "md",
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  size?: "sm" | "md";
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`pg-switch ${checked ? "on" : ""} ${disabled ? "disabled" : ""} ${size === "sm" ? "sm" : ""}`}
    >
      <span className="pg-switch-knob" />
    </button>
  );
}
