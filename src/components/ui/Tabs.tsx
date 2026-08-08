import React from "react";
import { cn } from "@/lib/utils";

interface TabsProps {
  tabs: { id: string; label: string; count?: number }[];
  activeTab: string;
  onChange: (id: string) => void;
  className?: string;
}

export function Tabs({ tabs, activeTab, onChange, className }: TabsProps) {
  return (
    <div className={cn("flex gap-1 border-b border-border-default", className)}>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onChange(tab.id)}
          className={cn(
            "px-4 py-2.5 text-sm font-medium transition-all duration-150 relative",
            "hover:text-text-primary",
            activeTab === tab.id
              ? "text-accent-default"
              : "text-text-dim",
          )}
        >
          <span className="inline-flex items-center gap-1.5">
            {tab.label}
            {tab.count !== undefined && (
              <span className={cn(
                "text-xs px-1.5 py-0.5 rounded-full",
                activeTab === tab.id
                  ? "bg-accent-subtle text-accent-default"
                  : "bg-gray-800 text-text-dim",
              )}>
                {tab.count}
              </span>
            )}
          </span>
          {activeTab === tab.id && (
            <span className="absolute bottom-0 left-0 right-0 h-0.5 bg-accent-default rounded-full" />
          )}
        </button>
      ))}
    </div>
  );
}
