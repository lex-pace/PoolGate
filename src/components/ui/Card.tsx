import React from "react";

export function Card({
  className = "", children, hover, glow, onClick,
}: {
  className?: string; children: React.ReactNode; hover?: boolean; glow?: "ok" | "err"; onClick?: () => void;
}) {
  return (
    <div
      className={`pg-panel p-4
        ${hover ? "hover:border-[var(--border-strong)] hover:-translate-y-px transition-all duration-200" : ""}
        ${glow === "ok" ? "card-glow-ok" : glow === "err" ? "card-glow-err" : ""}
        ${onClick ? "cursor-pointer" : ""}
        ${className}`}
      onClick={onClick}
    >
      {children}
    </div>
  );
}

export function CHeader({ className = "", children }: { className?: string; children: React.ReactNode }) {
  return <div className={`flex items-center justify-between mb-3 ${className}`}>{children}</div>;
}
export function CTitle({ className = "", children }: { className?: string; children: React.ReactNode }) {
  return <h3 className={`text-[12px] font-semibold tracking-[-0.01em] text-[var(--text-primary)] ${className}`}>{children}</h3>;
}
export function CBody({ className = "", children }: { className?: string; children: React.ReactNode }) {
  return <div className={className}>{children}</div>;
}
