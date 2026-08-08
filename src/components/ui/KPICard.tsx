import React from "react";

export function KPICard({ title, value, sub, trend, up, icon, onClick, className = "" }: {
  title: string; value: string; sub?: string; trend?: string; up?: boolean; icon?: React.ReactNode;
  onClick?: () => void; className?: string;
}) {
  return (
    <div
      className={`pg-panel p-4 transition-all duration-200
        ${onClick ? "cursor-pointer hover:border-[var(--border-strong)] hover:-translate-y-px" : ""}
        ${className}`}
      onClick={onClick}
    >
      <div className="flex items-center justify-between mb-3">
        <span className="pg-eyebrow">{title}</span>
        {icon && (
          <span className="flex items-center justify-center w-7 h-7 rounded-[7px] bg-[var(--bg-elevated)] text-[var(--text-dim)] border border-[var(--border-subtle)]">
            {icon}
          </span>
        )}
      </div>
      <div className="kpi-value text-[var(--text-primary)] animate-count-up">{value}</div>
      <div className="flex items-center gap-2 mt-2">
        {trend && (
          <span className={`text-xs font-medium inline-flex items-center gap-0.5
            ${up === true ? "text-[var(--color-ok)]" : up === false ? "text-[var(--color-err)]" : "text-[var(--text-dim)]"}`}>
            {up === true ? (
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none"><path d="M6 2.5L9.5 6.5H7.5V9.5H4.5V6.5H2.5L6 2.5Z" fill="currentColor"/></svg>
            ) : up === false ? (
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none"><path d="M6 9.5L2.5 5.5H4.5V2.5H7.5V5.5H9.5L6 9.5Z" fill="currentColor"/></svg>
            ) : null}
            {trend}
          </span>
        )}
        {sub && <span className="text-xs text-[var(--text-dim)]">{sub}</span>}
      </div>
    </div>
  );
}
