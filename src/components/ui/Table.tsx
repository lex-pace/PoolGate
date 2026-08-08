import React from "react";
import { cn } from "@/lib/utils";

interface TableProps {
  className?: string;
  children: React.ReactNode;
}

export function Table({ className, children }: TableProps) {
  return (
    <div className="w-full overflow-auto">
      <table className={cn("w-full text-sm", className)}>
        {children}
      </table>
    </div>
  );
}

export function TableHead({ className, children }: { className?: string; children: React.ReactNode }) {
  return (
    <thead className={cn("bg-gray-800/50", className)}>
      {children}
    </thead>
  );
}

export function TableBody({ className, children }: { className?: string; children: React.ReactNode }) {
  return <tbody className={cn("", className)}>{children}</tbody>;
}

export function TableRow({
  className,
  children,
  clickable,
  onClick,
}: {
  className?: string;
  children: React.ReactNode;
  clickable?: boolean;
  onClick?: () => void;
}) {
  return (
    <tr
      className={cn(
        "border-b border-border-default transition-colors duration-100",
        clickable && "cursor-pointer hover:bg-gray-800/30",
        className,
      )}
      onClick={onClick}
    >
      {children}
    </tr>
  );
}

export function TableCell({
  className,
  children,
  colSpan,
  style,
}: {
  className?: string;
  children: React.ReactNode;
  colSpan?: number;
  style?: React.CSSProperties;
}) {
  return <td className={cn("py-3 px-4 text-text-primary", className)} colSpan={colSpan} style={style}>{children}</td>;
}

export function TableHeaderCell({
  className,
  children,
  sortable,
  sortDir,
  onSort,
}: {
  className?: string;
  children: React.ReactNode;
  sortable?: boolean;
  sortDir?: "asc" | "desc" | null;
  onSort?: () => void;
}) {
  return (
    <th
      className={cn(
        "py-3 px-4 text-xs font-medium text-text-dim uppercase tracking-wider text-left",
        sortable && "cursor-pointer hover:text-text-secondary select-none",
        className,
      )}
      onClick={sortable ? onSort : undefined}
    >
      <span className="inline-flex items-center gap-1">
        {children}
        {sortable && (
          <span className="text-text-dim">
            {sortDir === "asc" ? "↑" : sortDir === "desc" ? "↓" : "↕"}
          </span>
        )}
      </span>
    </th>
  );
}
