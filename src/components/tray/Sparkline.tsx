/**
 * 轻量 SVG 折线/面积图（无依赖），托盘卡片共用。
 * 从 TrayCard 内联实现提取，Gateway 托盘与 Token Monitor 托盘复用。
 * 描边/填充默认走 CSS 类（.pg-tray-chart-line / .pg-tray-chart-area），
 * 也可用 stroke/fill 内联覆盖（Token Monitor 卡片跟随其主题色）。
 *
 * 刻意不使用 stroke-dasharray 做「描边绘制」动画：部分 WebView 在
 * preserveAspectRatio="none" 非等比缩放 + vector-effect: non-scaling-stroke
 * 下会把 dash 按设备单位解析，长路径中间会出现断点（多次修复无效）。
 * 动态效果改由 CSS 淡入（见 .pg-tray-sparkline 动画），折线恒为实线。
 */
export default function Sparkline({
  values,
  area = false,
  stroke,
  fill,
}: {
  values: number[];
  area?: boolean;
  stroke?: string;
  fill?: string;
}) {
  const safe = values.length ? values : Array.from({ length: 24 }, () => 0);
  const max = Math.max(1, ...safe);
  const points = safe.map((value, index) => {
    const x = safe.length === 1 ? 0 : (index / (safe.length - 1)) * 100;
    const y = 38 - (value / max) * 31;
    return [x, y] as const;
  });
  const line = points.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`).join(" ");
  const areaPath = `${line} L100 40 L0 40 Z`;

  return (
    <svg className="pg-tray-sparkline" viewBox="0 0 100 42" preserveAspectRatio="none" aria-hidden="true">
      <line x1="0" y1="9" x2="100" y2="9" className="pg-tray-grid-line" />
      <line x1="0" y1="24" x2="100" y2="24" className="pg-tray-grid-line" />
      <line x1="0" y1="39" x2="100" y2="39" className="pg-tray-grid-line" />
      {area && <path d={areaPath} className="pg-tray-chart-area" style={fill ? { fill } : undefined} />}
      <path d={line} className="pg-tray-chart-line" style={stroke ? { stroke } : undefined} />
    </svg>
  );
}
