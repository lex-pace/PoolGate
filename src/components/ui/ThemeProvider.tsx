import React, { createContext, useContext, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isTauriRuntime,
  readGlassNumbers,
  syncAppearancePrefs,
} from "@/lib/appearance";

export type ThemePreference = "system" | "light" | "dark";
type Theme = "dark" | "light";

interface ThemeCtx {
  theme: Theme;
  preference: ThemePreference;
  /** 设置主题偏好（随系统/浅色/深色）。持久化到 localStorage，跨窗口（主窗口 ↔ 托盘）实时同步。 */
  setPreference: (preference: ThemePreference) => void;
}

const THEME_KEY = "pg.theme";

const getSystemTheme = (): Theme => {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
};

const readPreference = (): ThemePreference => {
  try {
    const raw = window.localStorage.getItem(THEME_KEY);
    if (raw === "light" || raw === "dark") return raw;
  } catch {
    // 隐私模式/无 storage 环境：回退随系统
  }
  return "system";
};

/** 读取 localStorage 中的显式主题选择（light/dark），无显式选择时返回 null。 */
const readExplicitPreference = (): "light" | "dark" | null => {
  try {
    const raw = window.localStorage.getItem(THEME_KEY);
    return raw === "light" || raw === "dark" ? raw : null;
  } catch {
    return null;
  }
};

const Ctx = createContext<ThemeCtx>({ theme: "dark", preference: "system", setPreference: () => {} });

function applyResolved(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.dataset.appearance = theme;
  document.documentElement.style.colorScheme = theme;
}

function applyTheme(pref: ThemePreference): Theme {
  const next: Theme = pref === "system" ? getSystemTheme() : pref;
  applyResolved(next);
  return next;
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(readPreference);
  const [theme, setTheme] = useState<Theme>(() => applyTheme(readPreference()));

  useEffect(() => {
    // 本窗口初始化 + 系统主题变化（仅「随系统」时跟随）
    const onSystemChange = () => {
      if (readPreference() === "system") {
        setPreferenceState("system");
        setTheme(applyTheme("system"));
        // 随系统模式下系统主题变化 → 同步 Rust（Windows 原生 Acrylic tint 跟随）
        const glass = readGlassNumbers();
        syncAppearancePrefs("system", glass.glassOpacity, glass.glassBlur);
      }
    };
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    // 跨窗口同步：主窗口改主题 → 托盘窗口（同源 localStorage）收到 storage 事件实时跟随
    const onStorage = () => {
      const pref = readPreference();
      setPreferenceState(pref);
      setTheme(applyTheme(pref));
    };
    media.addEventListener?.("change", onSystemChange);
    window.addEventListener("storage", onStorage);

    // Rust 广播的外观主题事件（托盘窗口创建后由后端按 settings 表广播）。
    // localStorage 中的显式选择优先；仅当本窗口无显式选择（首次打开/新上下文）时，
    // 用 settings 表的偏好兜底校正，避免镜像覆盖用户的选择。
    let unlistenAppearance: (() => void) | undefined;
    if (isTauriRuntime()) {
      listen<{ theme?: string; preference?: string }>("appearance:theme", (event) => {
        if (readExplicitPreference() !== null) return; // 显式选择优先
        const pref: ThemePreference =
          event.payload.preference === "light" || event.payload.preference === "dark"
            ? event.payload.preference
            : "system";
        try {
          window.localStorage.setItem(THEME_KEY, pref);
        } catch {
          // ignore
        }
        setPreferenceState(pref);
        const resolved: Theme = event.payload.theme === "dark" ? "dark" : "light";
        applyResolved(resolved);
        setTheme(resolved);
      })
        .then((unlisten) => {
          unlistenAppearance = unlisten;
        })
        .catch(() => {
          // 浏览器预览/无事件系统：忽略
        });
    }

    // 一次性回写：老版本只把主题存于 localStorage，首次运行把显式选择镜像到
    // Rust settings 表（让 Windows 原生 tint 与广播在首轮即生效）。
    if (isTauriRuntime() && readExplicitPreference() !== null) {
      const glass = readGlassNumbers();
      syncAppearancePrefs(readPreference(), glass.glassOpacity, glass.glassBlur);
    }

    return () => {
      media.removeEventListener?.("change", onSystemChange);
      window.removeEventListener("storage", onStorage);
      unlistenAppearance?.();
    };
  }, []);

  const setPreference = (next: ThemePreference) => {
    try {
      window.localStorage.setItem(THEME_KEY, next);
    } catch {
      // ignore
    }
    setPreferenceState(next);
    setTheme(applyTheme(next));
    // 同步到 Rust settings 表 + Windows 原生 Acrylic tint
    const glass = readGlassNumbers();
    syncAppearancePrefs(next, glass.glassOpacity, glass.glassBlur);
  };

  return <Ctx.Provider value={{ theme, preference, setPreference }}>{children}</Ctx.Provider>;
}

export const useTheme = () => useContext(Ctx);
