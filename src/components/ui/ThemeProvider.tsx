import React, { createContext, useContext, useEffect, useState } from "react";

type Theme = "dark" | "light";

interface ThemeCtx {
  theme: Theme;
  followsSystem: true;
}

const getSystemTheme = (): Theme => {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
};

const Ctx = createContext<ThemeCtx>({ theme: "dark", followsSystem: true });

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setTheme] = useState<Theme>(getSystemTheme);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const syncTheme = (event?: MediaQueryListEvent) => {
      const nextTheme: Theme = (event?.matches ?? media.matches) ? "dark" : "light";
      setTheme(nextTheme);
      document.documentElement.dataset.theme = nextTheme;
      document.documentElement.dataset.appearance = nextTheme;
      document.documentElement.style.colorScheme = nextTheme;
    };

    syncTheme();
    media.addEventListener?.("change", syncTheme);
    return () => media.removeEventListener?.("change", syncTheme);
  }, []);

  return <Ctx.Provider value={{ theme, followsSystem: true }}>{children}</Ctx.Provider>;
}

export const useTheme = () => useContext(Ctx);
