import React from "react";

const badge: Record<string, string> = {
  ok: "bg-[var(--color-ok-bg)] text-[var(--color-ok)] border-[var(--color-ok)]/20",
  warn: "bg-[var(--color-warn-bg)] text-[var(--color-warn)] border-[var(--color-warn)]/20",
  err: "bg-[var(--color-err-bg)] text-[var(--color-err)] border-[var(--color-err)]/20",
  info: "bg-[var(--color-info-bg)] text-[var(--color-info)] border-[var(--color-info)]/20",
  mute: "bg-[var(--color-mute-bg)] text-[var(--color-mute)] border-[var(--color-mute)]/20",
  brand: "bg-[var(--color-brand-subtle)] text-[var(--color-brand)] border-[var(--color-brand)]/20",
};

export function Badge({ className = "", variant = "mute", dot, children }: {
  className?: string; variant?: keyof typeof badge; dot?: boolean; children: React.ReactNode;
}) {
  return (
    <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] leading-4 font-medium rounded-md border
      ${badge[variant] || badge.mute} ${className}`}>
      {dot && <span className={`w-1.5 h-1.5 rounded-full inline-block status-dot ${variant === "ok" ? "ok" : variant === "warn" ? "warn" : variant === "err" ? "err" : "mute"}`} />}
      {children}
    </span>
  );
}
