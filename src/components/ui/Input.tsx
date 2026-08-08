import React from "react";

export function Input({ className = "", label, error, ...rest }: React.InputHTMLAttributes<HTMLInputElement> & { label?: string; error?: string }) {
  return (
    <div className="flex flex-col gap-1">
      {label && <label className="text-xs font-medium text-[var(--text-secondary)]">{label}</label>}
      <input
        className={`h-9 px-3 text-sm rounded border bg-[var(--bg-surface)] text-[var(--text-primary)]
          border-[var(--border-default)] focus:border-[var(--color-brand)] focus:ring-1 focus:ring-[var(--color-brand)]
          outline-none transition-colors placeholder:text-[var(--text-dim)]
          ${error ? "border-[var(--color-err)]" : ""} ${className}`}
        {...rest}
      />
      {error && <span className="text-xs text-[var(--color-err)]">{error}</span>}
    </div>
  );
}

export function Select({ className = "", label, error, options, placeholder, ...rest }: React.SelectHTMLAttributes<HTMLSelectElement> & {
  label?: string; error?: string; options: { value: string; label: string }[]; placeholder?: string;
}) {
  return (
    <div className="flex flex-col gap-1">
      {label && <label className="text-xs font-medium text-[var(--text-secondary)]">{label}</label>}
      <select
        className={`h-9 px-3 text-sm rounded border bg-[var(--bg-surface)] text-[var(--text-primary)]
          border-[var(--border-default)] focus:border-[var(--color-brand)] outline-none transition-colors cursor-pointer
          ${error ? "border-[var(--color-err)]" : ""} ${className}`}
        {...rest}
      >
        {placeholder && <option value="">{placeholder}</option>}
        {options.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
      {error && <span className="text-xs text-[var(--color-err)]">{error}</span>}
    </div>
  );
}

export function Tabs({ tabs, active, onChange, className = "" }: {
  tabs: { id: string; label: string; count?: number }[];
  active: string; onChange: (id: string) => void; className?: string;
}) {
  return (
    <div className={`flex gap-1 border-b border-[var(--border-default)] ${className}`}>
      {tabs.map((t) => (
        <button key={t.id} onClick={() => onChange(t.id)}
          className={`px-3.5 py-2 text-sm font-medium relative transition-colors whitespace-nowrap
            ${active === t.id ? "text-[var(--color-brand)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}>
          {t.label}
          {t.count !== undefined && (
            <span className={`ml-1.5 text-[10px] px-1.5 py-0.5 rounded-full
              ${active === t.id ? "bg-[var(--color-brand-subtle)]" : "bg-[var(--bg-hover)]"}`}>{t.count}</span>
          )}
          {active === t.id && <span className="absolute bottom-0 left-2 right-2 h-0.5 bg-[var(--color-brand)] rounded-full" />}
        </button>
      ))}
    </div>
  );
}
