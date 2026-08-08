import React from "react";

const variants: Record<string, string> = {
  primary: "border border-[var(--color-brand)] bg-[var(--color-brand)] text-white shadow-sm hover:bg-[var(--color-brand-hover)]",
  secondary: "border border-[var(--border-default)] bg-[var(--bg-elevated)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]",
  outline: "border border-[var(--border-default)] bg-transparent text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)]",
  ghost: "border border-transparent text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]",
  danger: "border border-[var(--color-err)] bg-[var(--color-err)] text-white shadow-sm hover:brightness-95",
  success: "border border-[var(--color-ok)] bg-[var(--color-ok)] text-white shadow-sm hover:brightness-95",
};

const sizes: Record<string, string> = {
  sm: "h-7 px-2.5 text-[11px] rounded-[7px]",
  md: "h-8 px-3 text-xs rounded-[7px]",
  lg: "h-10 px-5 text-sm rounded-lg",
  icon: "w-8 h-8 rounded-[7px]",
};

interface BtnProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: keyof typeof variants;
  size?: keyof typeof sizes;
  loading?: boolean;
  fullWidth?: boolean;
}

export function Button({
  className = "",
  variant = "primary",
  size = "md",
  loading,
  disabled,
  fullWidth,
  children,
  ...rest
}: BtnProps) {
  const dis = disabled || loading;
  return (
    <button
      className={`inline-flex items-center justify-center gap-1.5 font-medium transition-all duration-150 select-none
        ${variants[variant] || variants.primary}
        ${sizes[size] || sizes.md}
        ${fullWidth ? "w-full" : ""}
        ${dis ? "opacity-40 cursor-not-allowed" : "active:brightness-95 cursor-pointer"}
        ${className}`}
      disabled={dis}
      {...rest}
    >
      {loading && (
        <svg className="animate-spin h-4 w-4 -ml-0.5" viewBox="0 0 24 24" fill="none">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
        </svg>
      )}
      {children}
    </button>
  );
}
