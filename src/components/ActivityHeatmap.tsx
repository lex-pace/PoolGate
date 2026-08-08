import React, { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { formatTokensZh } from "@/lib/utils";

export interface HeatmapDay {
  /** Calendar day, YYYY-MM-DD (local). */
  date: string;
  tokens: number;
  requests: number;
}

const BLUE = "10,132,255";
const LEVEL_BG = [
  `rgba(${BLUE}, 0.16)`,
  `rgba(${BLUE}, 0.32)`,
  `rgba(${BLUE}, 0.52)`,
  `rgba(${BLUE}, 0.76)`,
  `rgb(${BLUE})`,
];
const NO_ACTIVITY_BG = "var(--bg-inset)";

/** 0 = no activity; 1..5 = increasing intensity relative to the window max. */
export function heatLevel(tokens: number, maxTokens: number): number {
  if (tokens <= 0 || maxTokens <= 0) return 0;
  const ratio = tokens / maxTokens;
  if (ratio < 0.2) return 1;
  if (ratio < 0.4) return 2;
  if (ratio < 0.6) return 3;
  if (ratio < 0.8) return 4;
  return 5;
}

/** 「低 → 高」图例，可独立放在标题栏右侧。 */
export function HeatmapLegend({ compact = false }: { compact?: boolean }) {
  return (
    <div className={`pg-heatmap-legend ${compact ? "compact" : ""}`} aria-label="Tokens 活跃度图例：由低到高">
      <span>低</span>
      <div className="pg-heatmap-legend-scale" aria-hidden="true">
        {[NO_ACTIVITY_BG, ...LEVEL_BG].map((color, index) => (
          <i key={index} style={{ backgroundColor: color }} />
        ))}
      </div>
      <span>高</span>
    </div>
  );
}

function parseDay(date: string): Date {
  return new Date(`${date}T00:00:00`);
}

function formatDateKey(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function startOfWeek(date: Date): Date {
  const result = new Date(date);
  const weekday = (result.getDay() + 6) % 7;
  result.setDate(result.getDate() - weekday);
  result.setHours(0, 0, 0, 0);
  return result;
}

function formatDateZh(date: string): string {
  const day = parseDay(date);
  return `${day.getMonth() + 1}月${day.getDate()}日`;
}

interface GridCell {
  day: HeatmapDay | null;
  key: string;
}

interface MonthMarker {
  week: number;
  label: string;
}

export default function ActivityHeatmap({
  data,
  cellSize = 12,
  gap = 3,
  rounded = 3,
  showLegend = true,
  showLabels = false,
  maxWeeks,
  align = "start",
  compact = false,
  fill = false,
  className = "",
}: {
  data: HeatmapDay[];
  cellSize?: number;
  gap?: number;
  rounded?: number;
  showLegend?: boolean;
  showLabels?: boolean;
  maxWeeks?: number;
  align?: "start" | "center" | "end";
  compact?: boolean;
  fill?: boolean;
  className?: string;
}) {
  const gridRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const [hover, setHover] = useState<{ x: number; y: number; width: number; day: HeatmapDay; clientX: number; clientY: number } | null>(null);

  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const updateWidth = () => setContainerWidth(node.clientWidth);
    updateWidth();
    const observer = new ResizeObserver(updateWidth);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  const visibleData = useMemo(() => {
    const sorted = [...data].sort((a, b) => a.date.localeCompare(b.date));
    if (!maxWeeks || !sorted.length) return sorted;

    const dataMap = new Map(sorted.map((day) => [day.date, day]));
    const latest = parseDay(sorted[sorted.length - 1].date);
    const windowStart = startOfWeek(latest);
    windowStart.setDate(windowStart.getDate() - (maxWeeks - 1) * 7);

    return Array.from({ length: maxWeeks * 7 }, (_, index) => {
      const date = new Date(windowStart);
      date.setDate(windowStart.getDate() + index);
      const key = formatDateKey(date);
      return dataMap.get(key) || { date: key, tokens: 0, requests: 0 };
    });
  }, [data, maxWeeks]);

  const layout = useMemo(() => {
    if (!visibleData.length) return { pad: 0, weeks: 0 };
    const pad = (parseDay(visibleData[0].date).getDay() + 6) % 7;
    const weeks = Math.ceil((visibleData.length + pad) / 7);
    return { pad, weeks };
  }, [visibleData]);

  const cells = useMemo(() => {
    if (!visibleData.length) return [] as GridCell[];
    const cells: GridCell[] = [];
    for (let week = 0; week < layout.weeks; week++) {
      for (let weekday = 0; weekday < 7; weekday++) {
        const index = week * 7 + weekday - layout.pad;
        if (index >= 0 && index < visibleData.length) {
          cells.push({ day: visibleData[index], key: visibleData[index].date });
        } else {
          cells.push({ day: null, key: `empty-${week}-${weekday}` });
        }
      }
    }
    return cells;
  }, [visibleData, layout]);

  const months = useMemo(() => {
    const markers: MonthMarker[] = [];
    let previousMonth = -1;
    for (let week = 0; week < layout.weeks; week++) {
      const day = cells[week * 7]?.day || cells[week * 7 + 1]?.day;
      if (!day) continue;
      const month = parseDay(day.date).getMonth();
      if (month !== previousMonth) {
        markers.push({ week, label: `${month + 1}月` });
        previousMonth = month;
      }
    }
    return markers;
  }, [cells, layout.weeks]);

  const maxTokens = useMemo(() => visibleData.reduce((max, day) => Math.max(max, day.tokens), 0), [visibleData]);
  const labelWidth = showLabels ? 22 : 0;
  const focusSafeArea = compact ? 4 : 6;
  const availableGridWidth = Math.max(0, containerWidth - labelWidth - focusSafeArea * 2);
  const fittedCellSize = fill && layout.weeks > 0
    ? Math.max(3, (availableGridWidth - Math.max(0, layout.weeks - 1) * gap) / layout.weeks)
    : cellSize;
  const fittedGap = fill && layout.weeks > 0 && fittedCellSize <= 4 ? 1 : gap;
  const resolvedCellSize = fill && layout.weeks > 0
    ? Math.max(3, (availableGridWidth - Math.max(0, layout.weeks - 1) * fittedGap) / layout.weeks)
    : cellSize;
  const gridWidth = layout.weeks * resolvedCellSize + Math.max(0, layout.weeks - 1) * fittedGap;

  const handleMove = (day: HeatmapDay, event: React.MouseEvent<HTMLButtonElement>) => {
    const rect = gridRef.current?.getBoundingClientRect();
    if (!rect) return;
    setHover({
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
      width: rect.width,
      clientX: event.clientX,
      clientY: event.clientY,
      day,
    });
  };

  const tooltipWidth = compact ? 190 : 240;
  const tooltipLeft = hover ? Math.max(4, Math.min(hover.x + 10, hover.width - tooltipWidth - 4)) : 0;
  const tooltipTop = hover ? Math.max(0, hover.y - (compact ? 35 : 42)) : 0;

  return (
    <div className={`pg-heatmap ${compact ? "compact" : ""} ${className}`}>
      {showLegend && <HeatmapLegend compact={compact} />}
      <div ref={scrollRef} className={`pg-heatmap-scroll align-${align} ${fill ? "fill" : ""}`}>
        <div className="pg-heatmap-layout">
          {showLabels && (
            <div className="pg-heatmap-weekdays" style={{ gap: fittedGap }} aria-hidden="true">
              {["一", "", "三", "", "五", "", "日"].map((label, index) => (
                <span key={index} style={{ height: resolvedCellSize, lineHeight: `${resolvedCellSize}px` }}>{label}</span>
              ))}
            </div>
          )}
          <div ref={gridRef} className="pg-heatmap-grid-wrap" style={{ padding: focusSafeArea }}>
            {showLabels && (
              <div className="pg-heatmap-months" style={{ width: gridWidth, height: 16 }} aria-hidden="true">
                {months.map((month) => (
                  <span key={`${month.week}-${month.label}`} style={{ left: month.week * (resolvedCellSize + fittedGap) }}>{month.label}</span>
                ))}
              </div>
            )}
            <div
              className="pg-heatmap-grid"
              style={{
                gridTemplateRows: `repeat(7, ${resolvedCellSize}px)`,
                gridTemplateColumns: `repeat(${layout.weeks || 1}, ${resolvedCellSize}px)`,
                gap: fittedGap,
              }}
            >
              {cells.map((cell) => {
                const day = cell.day;
                if (!day) return <span key={cell.key} className="pg-heatmap-empty" />;
                const level = heatLevel(day.tokens, maxTokens);
                const label = `${formatDateZh(day.date)}，${formatTokensZh(day.tokens)} Tokens，${day.requests} 次请求`;
                return (
                  <button
                    key={cell.key}
                    type="button"
                    className="pg-heatmap-cell"
                    aria-label={label}
                    title={compact ? label : undefined}
                    onMouseEnter={(event) => handleMove(day, event)}
                    onMouseMove={(event) => handleMove(day, event)}
                    onMouseLeave={() => setHover(null)}
                    onFocus={(event) => {
                      const rect = event.currentTarget.getBoundingClientRect();
                      const gridRect = gridRef.current?.getBoundingClientRect();
                      if (gridRect) setHover({
                        x: rect.left - gridRect.left,
                        y: rect.top - gridRect.top,
                        width: gridRect.width,
                        clientX: rect.left + rect.width / 2,
                        clientY: rect.top,
                        day,
                      });
                    }}
                    onBlur={() => setHover(null)}
                    style={{
                      width: resolvedCellSize,
                      height: resolvedCellSize,
                      borderRadius: rounded,
                      backgroundColor: level === 0 ? NO_ACTIVITY_BG : LEVEL_BG[level - 1],
                    }}
                  />
                );
              })}
            </div>
            {hover && !compact && (
              <div className="pg-heatmap-tooltip" style={{ left: tooltipLeft, top: tooltipTop }}>
                <strong>{formatDateZh(hover.day.date)}</strong>
                <span>{formatTokensZh(hover.day.tokens)} Tokens</span>
                <em>{hover.day.requests} 次请求</em>
              </div>
            )}
            {hover && compact && createPortal(
              <div
                className="pg-heatmap-tooltip compact"
                role="tooltip"
                style={{
                  left: Math.max(8, Math.min(hover.clientX + 10, window.innerWidth - 170)),
                  top: Math.max(8, Math.min(hover.clientY - 38, window.innerHeight - 34)),
                }}
              >
                <strong>{formatDateZh(hover.day.date)}</strong>
                <span>{formatTokensZh(hover.day.tokens)} Tokens</span>
                <em>{hover.day.requests} 次</em>
              </div>,
              document.body,
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
