import React, { useEffect } from "react";

export function Modal({ open, onClose, title, children, className = "", contentClassName = "", placement = "center" }: {
  open: boolean; onClose: () => void; title?: string; children: React.ReactNode; className?: string; contentClassName?: string; placement?: "center" | "right";
}) {
  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", h);
    return () => document.removeEventListener("keydown", h);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className={`fixed inset-0 z-[110] flex ${placement === "right" ? "items-stretch justify-end" : "items-center justify-center"}`}>
      <div className="absolute inset-0 bg-black/45 backdrop-blur-[2px] animate-fade-in" onClick={onClose} />
      <div
        className={`relative bg-[var(--bg-elevated)] border border-[var(--border-default)] w-full flex flex-col overflow-hidden ${placement === "right" ? "h-full max-h-none rounded-l-xl border-y-0 border-r-0 animate-slide-left" : "rounded-lg max-w-lg mx-4 max-h-[85vh] animate-slide-up"} ${className}`}
        style={{ boxShadow: "var(--shadow-elevated)" }}
      >
        {title && (
          <div className="flex items-center justify-between px-5 py-3.5 border-b border-[var(--border-default)]">
            <h2 className="text-base font-semibold text-[var(--text-primary)]">{title}</h2>
            <button onClick={onClose} className="text-[var(--text-dim)] hover:text-[var(--text-primary)] transition-colors text-lg leading-none">✕</button>
          </div>
        )}
        <div className={`flex-1 min-h-0 overflow-auto p-5 ${contentClassName}`}>{children}</div>
      </div>
    </div>
  );
}

export function Spinner({ className = "" }: { className?: string }) {
  return (
    <svg className={`animate-spin h-5 w-5 text-[var(--color-brand)] ${className}`} viewBox="0 0 24 24" fill="none">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
    </svg>
  );
}
