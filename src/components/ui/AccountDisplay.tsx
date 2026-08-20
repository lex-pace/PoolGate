import React, { createContext, useContext, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isTauriRuntime,
  readAccountDisplay,
  readGlassNumbers,
  readThemePreference,
  syncAccountDisplayPrefs,
  syncAppearancePrefs,
  type AccountDisplayPreference,
} from "@/lib/appearance";

/**
 * 额度账号身份展示设置：脱密显示 / 完整显示。
 *
 * - 脱密显示（默认）：账号名只取「可读名称」，身份一律脱敏（邮箱掩码、密钥只留首尾）；
 * - 完整显示：按原样展示账号 label / 邮箱等完整身份；
 * - 持久化两处：localStorage（`pg.accountDisplay`，CSS/渲染即时生效，跨窗口经 storage 事件
 *   实时同步）+ Rust settings 表（与主题/玻璃同源，托盘窗口创建后由后端广播
 *   `appearance:theme` 事件兑底校正，首次打开即生效）；
 * - 桌面端设置页可切换，同时控制桌面端与托盘所有展示账号的地方。
 */
export type AccountDisplayMode = AccountDisplayPreference;

export interface AccountDisplayCtxValue {
  mode: AccountDisplayMode;
  setMode: (mode: AccountDisplayMode) => void;
}

const KEY = "pg.accountDisplay";
const DEFAULT_MODE: AccountDisplayMode = "mask";

const readMode = (): AccountDisplayMode => readAccountDisplay();

/** 读取 localStorage 中的显式选择，无显式选择返回 null（用于「显式选择优先」判定）。 */
const readExplicitMode = (): AccountDisplayMode | null => {
  try {
    const raw = window.localStorage.getItem(KEY);
    return raw === "full" || raw === "mask" ? raw : null;
  } catch {
    return null;
  }
};

const Ctx = createContext<AccountDisplayCtxValue>({
  mode: DEFAULT_MODE,
  setMode: () => {},
});

export function AccountDisplayProvider({ children }: { children: React.ReactNode }) {
  const [mode, setModeState] = useState<AccountDisplayMode>(readMode);

  // 本窗口初始化 + 跨窗口 storage 同步（主窗口改 → 托盘实时跟随）
  useEffect(() => {
    setModeState(readMode());
    const onStorage = () => setModeState(readMode());
    window.addEventListener("storage", onStorage);

    // Rust 广播的外观事件（托盘窗口创建后由后端按 settings 表广播，载荷含
    // accountDisplay）。localStorage 中的显式选择优先；仅当本窗口无显式选择
    // （首次打开/新上下文）时用 settings 表的偏好兜底校正，避免镜像覆盖用户选择。
    let unlistenAppearance: (() => void) | undefined;
    if (isTauriRuntime()) {
      listen<{ accountDisplay?: string }>("appearance:theme", (event) => {
        if (readExplicitMode() !== null) return; // 显式选择优先
        const next: AccountDisplayMode =
          event.payload.accountDisplay === "full" || event.payload.accountDisplay === "mask"
            ? event.payload.accountDisplay
            : DEFAULT_MODE;
        try {
          window.localStorage.setItem(KEY, next);
        } catch {
          // ignore
        }
        setModeState(next);
      })
        .then((unlisten) => {
          unlistenAppearance = unlisten;
        })
        .catch(() => {
          // 浏览器预览/无事件系统：忽略
        });
    }

    // 一次性回写：老版本只存于 localStorage，首次运行把显式选择镜像到 Rust
    // settings 表（让托盘首轮广播即带上持久化偏好）。
    if (isTauriRuntime() && readExplicitMode() !== null) {
      const glass = readGlassNumbers();
      syncAppearancePrefs(
        readThemePreference(),
        glass.glassOpacity,
        glass.glassBlur,
        readExplicitMode() ?? DEFAULT_MODE,
      );
    }

    return () => {
      window.removeEventListener("storage", onStorage);
      unlistenAppearance?.();
    };
  }, []);

  const setMode = (next: AccountDisplayMode) => {
    try {
      window.localStorage.setItem(KEY, next);
    } catch {
      // 隐私模式/无 storage：仅本次生效
    }
    setModeState(next);
    // 同步到 Rust settings 表（与主题/玻璃同源持久化，托盘首开即生效）
    syncAccountDisplayPrefs(next);
  };

  return <Ctx.Provider value={{ mode, setMode }}>{children}</Ctx.Provider>;
}

export const useAccountDisplay = () => useContext(Ctx);
