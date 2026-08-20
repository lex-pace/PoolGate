import React, { createContext, useContext, useEffect, useState } from "react";
import {
  isTauriRuntime,
  readThemePreference,
  syncAppearancePrefs,
} from "@/lib/appearance";

/**
 * 玻璃效果设置（对齐开源 Token Monitor Appearance 的 glassOpacity / glassBlur）。
 * 同一组 CSS 变量同时驱动深色玻璃层——托盘外壳（.pg-tray-glass）、主窗口侧栏/工具栏
 * （.pg-sidebar / .pg-toolbar，可选跟随）与 Token Monitor 仪表盘卡片
 * （.pg-tm-dash .pg-panel）。
 *
 * - glassOpacity 0-100 → `--tg-glass-alpha`（玻璃层背景透明度）；
 * - glassBlur 0-100px → `--tg-glass-blur`（backdrop-filter blur 像素）；
 * - chromeFollows：主窗口侧栏/工具栏跟随全局玻璃滑块（默认开启，托盘与桌面端统一）；
 * - 持久化到 localStorage（`pg.glassOpacity` / `pg.glassBlur` / `pg.glassChrome`），
 *   跨窗口（主窗口 ↔ 托盘）经 storage 事件实时同步；
 * - 未自定义时采用 iOS 26 风格默认（低 alpha、64px blur）；重置即清除覆盖并恢复
 *   默认开关状态。
 */

export interface GlassSettings {
  glassOpacity: number;
  glassBlur: number;
  /** 主窗口侧栏/工具栏是否跟随玻璃滑块（默认开启）。 */
  chromeFollows: boolean;
  setGlassSettings: (opacity: number, blur: number) => void;
  setChromeFollows: (follows: boolean) => void;
  resetGlassSettings: () => void;
}

const OPACITY_KEY = "pg.glassOpacity";
const BLUR_KEY = "pg.glassBlur";
const CHROME_KEY = "pg.glassChrome";
const DEFAULT_OPACITY = 32;
const DEFAULT_BLUR = 64;
const DEFAULT_CHROME_FOLLOWS = true;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

const readNumber = (key: string, fallback: number): number => {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw == null) return fallback;
    const n = Number(raw);
    return Number.isFinite(n) ? n : fallback;
  } catch {
    return fallback;
  }
};

const readChromeFollows = (): boolean => {
  try {
    return window.localStorage.getItem(CHROME_KEY) === "1";
  } catch {
    return DEFAULT_CHROME_FOLLOWS;
  }
};

/** 把玻璃设置写入 CSS 变量（对齐开源 applyAppearance 的 setProperty 模式）。 */
function applyGlass(opacity: number, blur: number) {
  const root = document.documentElement.style;
  root.setProperty("--tg-glass-alpha", (clamp(opacity, 0, 100) / 100).toFixed(2));
  root.setProperty("--tg-glass-blur", `${clamp(blur, 0, 100)}px`);
}

/** 侧栏/工具栏跟随开关 → `data-glass-chrome`（CSS 据此选择变量驱动或固定默认）。 */
function applyChrome(follows: boolean) {
  document.documentElement.dataset.glassChrome = follows ? "on" : "off";
}

const Ctx = createContext<GlassSettings>({
  glassOpacity: DEFAULT_OPACITY,
  glassBlur: DEFAULT_BLUR,
  chromeFollows: DEFAULT_CHROME_FOLLOWS,
  setGlassSettings: () => {},
  setChromeFollows: () => {},
  resetGlassSettings: () => {},
});

export function GlassSettingsProvider({ children }: { children: React.ReactNode }) {
  const [glassOpacity, setOpacity] = useState(() => readNumber(OPACITY_KEY, DEFAULT_OPACITY));
  const [glassBlur, setBlur] = useState(() => readNumber(BLUR_KEY, DEFAULT_BLUR));
  const [chromeFollows, setChromeFollowsState] = useState(readChromeFollows);

  // 本窗口初始化（含跨窗口 storage 同步：主窗口改 → 托盘实时跟随）
  useEffect(() => {
    applyGlass(readNumber(OPACITY_KEY, DEFAULT_OPACITY), readNumber(BLUR_KEY, DEFAULT_BLUR));
    applyChrome(readChromeFollows());
    // 一次性回写：老版本只把玻璃设置存于 localStorage；仅当确实自定义过（键存在）
    // 才镜像到 Rust settings 表——避免写入默认值覆盖后端「未自定义」的语义。
    const customized =
      window.localStorage.getItem(OPACITY_KEY) != null ||
      window.localStorage.getItem(BLUR_KEY) != null;
    if (isTauriRuntime() && customized) {
      syncAppearancePrefs(
        readThemePreference(),
        readNumber(OPACITY_KEY, DEFAULT_OPACITY),
        readNumber(BLUR_KEY, DEFAULT_BLUR),
      );
    }
    const onStorage = () => {
      setOpacity(readNumber(OPACITY_KEY, DEFAULT_OPACITY));
      setBlur(readNumber(BLUR_KEY, DEFAULT_BLUR));
      setChromeFollowsState(readChromeFollows());
      applyChrome(readChromeFollows());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const setGlassSettings = (opacity: number, blur: number) => {
    const o = clamp(opacity, 0, 100);
    const b = clamp(blur, 0, 100);
    try {
      window.localStorage.setItem(OPACITY_KEY, String(o));
      window.localStorage.setItem(BLUR_KEY, String(b));
    } catch {
      // 隐私模式/无 storage：仅本次生效
    }
    setOpacity(o);
    setBlur(b);
    applyGlass(o, b);
    // 同步到 Rust settings 表 + Windows 原生 Acrylic tint（alpha 跟随不透明度）
    syncAppearancePrefs(readThemePreference(), o, b);
  };

  const setChromeFollows = (follows: boolean) => {
    try {
      window.localStorage.setItem(CHROME_KEY, follows ? "1" : "0");
    } catch {
      // ignore
    }
    setChromeFollowsState(follows);
    applyChrome(follows);
  };

  const resetGlassSettings = () => {
    try {
      window.localStorage.removeItem(OPACITY_KEY);
      window.localStorage.removeItem(BLUR_KEY);
      window.localStorage.removeItem(CHROME_KEY);
    } catch {
      // ignore
    }
    // 清除覆盖 → CSS 恢复 iOS 26 风格默认；开关恢复为桌面端跟随玻璃滑块
    document.documentElement.style.removeProperty("--tg-glass-alpha");
    document.documentElement.style.removeProperty("--tg-glass-blur");
    applyChrome(DEFAULT_CHROME_FOLLOWS);
    setOpacity(DEFAULT_OPACITY);
    setBlur(DEFAULT_BLUR);
    setChromeFollowsState(DEFAULT_CHROME_FOLLOWS);
    // 同步恢复默认到 Rust（写入默认值等价于删除，后端 alpha 回退各主题默认）
    syncAppearancePrefs(readThemePreference(), DEFAULT_OPACITY, DEFAULT_BLUR);
  };

  return (
    <Ctx.Provider
      value={{
        glassOpacity,
        glassBlur,
        chromeFollows,
        setGlassSettings,
        setChromeFollows,
        resetGlassSettings,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export const useGlassSettings = () => useContext(Ctx);
