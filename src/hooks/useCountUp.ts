import { useEffect, useRef, useState } from "react";

/**
 * 数字滚动动画（对齐开源 Token Monitor：数值变化时 count-up）。
 * 首次挂载从 0 滚到目标值；后续变化从旧值缓动到新值。
 * 返回当前帧的整数值，调用方自行格式化（千分位 / 万 / 亿 等）。
 */
export default function useCountUp(target: number, duration = 700): number {
  const [value, setValue] = useState(0);
  const prevRef = useRef(0);
  useEffect(() => {
    const from = prevRef.current;
    if (from === target) {
      setValue(target);
      return;
    }
    prevRef.current = target;
    const start = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      const eased = 1 - Math.pow(1 - t, 3);
      setValue(Math.round(from + (target - from) * eased));
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target, duration]);
  return value;
}
