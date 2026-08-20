import React from "react";

/**
 * 分段切换控件（iOS 风格：浅色轨道 + 浮起的活动段），用于 Token Monitor 仪表盘
 * 内所有选择切换（今日/本月/累计、用量/成本、按工具/按模型、时间范围、tab 导航）。
 */
export default function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  size = "sm",
  className = "",
}: {
  options: Array<{ value: T; label: string }>;
  value: T;
  onChange: (v: T) => void;
  size?: "sm" | "md";
  className?: string;
}) {
  return (
    <div className={`pg-seg ${size === "md" ? "pg-seg-md" : ""} ${className}`} role="tablist">
      {options.map((option) => (
        <button
          key={option.value}
          role="tab"
          aria-selected={value === option.value}
          className={`pg-seg-item ${value === option.value ? "active" : ""}`}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
