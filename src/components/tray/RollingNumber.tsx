import { useEffect, useRef, useState } from "react";

/**
 * 里程表式滚动数字（翻牌效果）：每个数字位是一个滚轮（0–9 竖直条带），
 * 数值变化时各滚轮以 CSS transform 平滑滚到新数字（低位先动，高位级联跟随）。
 * - 数字位数变化时整体重新挂载，所有滚轮从 0 滚到目标（增长场景观感最佳）。
 * - 支持自定义 format（如 "1,234,567"、"638.4万"）：非数字字符作为静态分隔符。
 * - 尊重 prefers-reduced-motion：关闭滚动直接显示目标值。
 */
export default function RollingNumber({
  value,
  format,
  duration = 480,
  className = "",
}: {
  value: number;
  /** 显示格式化；默认整数千分位（en-US）。返回串中的数字字符逐一滚动。 */
  format?: (n: number) => string;
  /** 单次滚轮滚动时长（ms）。 */
  duration?: number;
  className?: string;
}) {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(mq.matches);
    update();
    mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, []);

  const str = (format ?? ((n: number) => Math.max(0, Math.round(n)).toLocaleString("en-US")))(value);
  const digitCount = str.replace(/\D/g, "").length;

  return (
    <span
      className={`pg-odo ${className}`}
      role="img"
      aria-label={str}
      aria-live="polite"
    >
      {/* 位数变化 → 重新挂载，所有滚轮从 0 滚到目标 */}
      <span key={digitCount} className="pg-odo-row">
        {[...str].map((ch, i) => {
          const digit = Number(ch);
          if (Number.isNaN(digit)) {
            return (
              <span key={i} className="pg-odo-sep" aria-hidden>
                {ch}
              </span>
            );
          }
          // 级联延迟：低位（右侧）先滚，高位依次跟随
          const digitIndex = str.slice(0, i).replace(/\D/g, "").length;
          const delay = (digitCount - 1 - digitIndex) * 35;
          return (
            <OdoCell
              key={i}
              digit={digit}
              duration={duration}
              delay={delay}
              reduced={reduced}
            />
          );
        })}
      </span>
    </span>
  );
}

/** 单个数字位滚轮：0–9 竖直条带，translateY 定位当前数字。 */
function OdoCell({
  digit,
  duration,
  delay,
  reduced,
}: {
  digit: number;
  duration: number;
  delay: number;
  reduced: boolean;
}) {
  const [shown, setShown] = useState(0);
  const first = useRef(true);

  useEffect(() => {
    if (first.current) {
      first.current = false;
      // 挂载：先画在 0，下一帧再滚到目标（首帧无 transition，双 rAF 触发滚动）
      const raf = requestAnimationFrame(() => {
        requestAnimationFrame(() => setShown(digit));
      });
      return () => cancelAnimationFrame(raf);
    }
    // 数值变化：从当前显示位滚到新位
    setShown(digit);
  }, [digit]);

  const transition = reduced
    ? "none"
    : `transform ${duration}ms cubic-bezier(.22,.61,.36,1) ${delay}ms`;

  return (
    <span className="pg-odo-cell" aria-hidden>
      <span
        className="pg-odo-strip"
        style={{ transform: `translateY(-${shown * 10}%)`, transition }}
      >
        {[0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map((d) => (
          <span key={d} className="pg-odo-digit">
            {d}
          </span>
        ))}
      </span>
    </span>
  );
}
