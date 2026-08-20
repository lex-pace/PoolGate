import type { ReactNode } from "react";

/**
 * 轻量 SVG 环形进度（无依赖），托盘卡片共用。
 * 设计规范 §8：Diameter 44 / Stroke 6，蓝 #007AFF / 绿 #34C759。
 * 供网关「资源使用」Mini Card 与 Token 页额度环复用；中心可放数字/图标（children）。
 */
export default function RingProgress({
  value,
  size = 44,
  stroke = 6,
  color = "var(--tg-primary)",
  track = "rgba(60,60,67,.12)",
  children,
}: {
  /** 0–100，超出自动夹取。 */
  value: number;
  size?: number;
  stroke?: number;
  color?: string;
  track?: string;
  children?: ReactNode;
}) {
  const clamped = Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const dash = (clamped / 100) * circumference;

  return (
    <span className="pg-ring" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={track}
          strokeWidth={stroke}
        />
        <circle
          className="pg-ring-value"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={color}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${dash} ${circumference}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      {children != null && <span className="pg-ring-center">{children}</span>}
    </span>
  );
}
