import React from "react";
import { cn } from "@/lib/utils";

interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  error?: string;
  options: { value: string; label: string }[];
  placeholder?: string;
}

export function Select({ className, label, error, options, placeholder, id, ...props }: SelectProps) {
  return (
    <div className="flex flex-col gap-1.5">
      {label && (
        <label htmlFor={id} className="text-sm text-text-secondary font-medium">
          {label}
        </label>
      )}
      <select
        id={id}
        className={cn(
          "h-9 px-3 text-sm rounded-md border bg-bg-surface text-text-primary",
          "border-border-default focus:border-accent-default focus:ring-1 focus:ring-accent-default",
          "transition-colors duration-150 outline-none cursor-pointer",
          error && "border-status-red",
          className,
        )}
        {...props}
      >
        {placeholder && <option value="">{placeholder}</option>}
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
      {error && <span className="text-xs text-status-red">{error}</span>}
    </div>
  );
}
