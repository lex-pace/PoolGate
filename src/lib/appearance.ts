import { invoke } from "@tauri-apps/api/core";

/**
 * 外观偏好（主题 + 托盘玻璃）的 Rust 同步助手。
 *
 * 前端把偏好同时写入两处：
 * - localStorage（`pg.theme` / `pg.glassOpacity` / `pg.glassBlur`）——CSS 立即生效，
 *   且跨窗口（主窗口 ↔ 托盘）经 `storage` 事件实时同步；
 * - Rust settings 表（同键名）——Windows 托盘的原生 Acrylic tint 需要 Rust 可读的
 *   持久化，切换主题时后端会重设 tint。
 *
 * 浏览器预览（无 Tauri 运行时）时静默跳过，不影响开发体验。
 */

export type ThemePreference = "system" | "light" | "dark";

export type AccountDisplayPreference = "mask" | "full";

const THEME_KEY = "pg.theme";
const OPACITY_KEY = "pg.glassOpacity";
const BLUR_KEY = "pg.glassBlur";
const ACCOUNT_DISPLAY_KEY = "pg.accountDisplay";

const DEFAULT_OPACITY = 32;
const DEFAULT_BLUR = 64;

/** 读取 localStorage 中的账号脱敏/全展示偏好，无显式选择回退脱密显示。 */
export function readAccountDisplay(): AccountDisplayPreference {
  try {
    if (window.localStorage.getItem(ACCOUNT_DISPLAY_KEY) === "full") return "full";
  } catch {
    // 隐私模式/无 storage 环境：回退脱密显示
  }
  return "mask";
}

/** Tauri 2 运行时存在时返回 true（`@tauri-apps/api` 的 invoke 依赖该全局对象）。 */
export const isTauriRuntime = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function readThemePreference(): ThemePreference {
  try {
    const raw = window.localStorage.getItem(THEME_KEY);
    if (raw === "light" || raw === "dark") return raw;
  } catch {
    // 隐私模式/无 storage 环境：回退随系统
  }
  return "system";
}

export function readGlassNumbers(): { glassOpacity: number; glassBlur: number } {
  const read = (key: string, fallback: number): number => {
    try {
      const raw = window.localStorage.getItem(key);
      if (raw == null) return fallback;
      const n = Number(raw);
      return Number.isFinite(n) ? n : fallback;
    } catch {
      return fallback;
    }
  };
  return {
    glassOpacity: read(OPACITY_KEY, DEFAULT_OPACITY),
    glassBlur: read(BLUR_KEY, DEFAULT_BLUR),
  };
}

/**
 * 把当前偏好同步到 Rust：写入 settings 表，Windows 上同时重设托盘原生 Acrylic tint。
 * 仅在 Tauri 运行时生效；失败只告警（CSS 外观不受影响）。
 */
export function syncAppearancePrefs(
  theme: ThemePreference,
  glassOpacity: number,
  glassBlur: number,
  accountDisplay?: AccountDisplayPreference,
): void {
  if (!isTauriRuntime()) return;
  invoke("set_appearance_prefs", {
    theme,
    glassOpacity,
    glassBlur,
    accountDisplay: accountDisplay ?? null,
  }).catch((error) => {
    console.warn("Sync appearance prefs to backend failed:", error);
  });
}

/** 仅同步账号脱敏/全展示偏好到 Rust（主题/玻璃走原调用，不动其他键）。 */
export function syncAccountDisplayPrefs(mode: AccountDisplayPreference): void {
  if (!isTauriRuntime()) return;
  invoke("set_appearance_prefs", {
    theme: readThemePreference(),
    glassOpacity: null,
    glassBlur: null,
    accountDisplay: mode,
  }).catch((error) => {
    console.warn("Sync account display pref to backend failed:", error);
  });
}
