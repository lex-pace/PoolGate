import React, { createContext, useContext, useState, useCallback, useRef, useEffect } from "react";
import { X, Check, AlertTriangle, Info } from "lucide-react";

type ToastType = "success" | "error" | "warning" | "info";

interface Toast {
  id: number;
  type: ToastType;
  message: string;
  duration?: number;
}

interface ToastCtx {
  toast: (type: ToastType, message: string, duration?: number) => void;
}

const Ctx = createContext<ToastCtx>({ toast: () => {} });

export function useToast() {
  return useContext(Ctx);
}

let nextId = 0;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timerRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const remove = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
    const timer = timerRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timerRef.current.delete(id);
    }
  }, []);

  const add = useCallback(
    (type: ToastType, message: string, duration = 3000) => {
      const id = nextId++;
      setToasts((prev) => [...prev, { id, type, message, duration }]);
      if (duration > 0) {
        const timer = setTimeout(() => remove(id), duration);
        timerRef.current.set(id, timer);
      }
    },
    [remove]
  );

  useEffect(() => {
    return () => {
      timerRef.current.forEach((t) => clearTimeout(t));
    };
  }, []);

  const typeConfig: Record<ToastType, { icon: React.ReactNode; bg: string; border: string; text: string }> = {
    success: { icon: <Check size={14} />, bg: "var(--color-ok-bg)", border: "var(--color-ok)", text: "var(--color-ok)" },
    error: { icon: <X size={14} />, bg: "var(--color-err-bg)", border: "var(--color-err)", text: "var(--color-err)" },
    warning: { icon: <AlertTriangle size={14} />, bg: "var(--color-warn-bg)", border: "var(--color-warn)", text: "var(--color-warn)" },
    info: { icon: <Info size={14} />, bg: "var(--color-info-bg)", border: "var(--color-info)", text: "var(--color-info)" },
  };

  return (
    <Ctx.Provider value={{ toast: add }}>
      {children}
      <div className="fixed bottom-12 right-4 z-[100] flex flex-col gap-2 pointer-events-none">
        {toasts.map((t) => {
          const cfg = typeConfig[t.type];
          return (
            <div
              key={t.id}
              className="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-lg border animate-slide-up max-w-sm"
              style={{
                backgroundColor: "var(--bg-elevated)",
                borderColor: cfg.border,
                boxShadow: "var(--shadow-elevated)",
              }}
            >
              <span
                className="flex items-center justify-center w-6 h-6 rounded-full shrink-0"
                style={{ backgroundColor: cfg.bg, color: cfg.text }}
              >
                {cfg.icon}
              </span>
              <span className="text-sm flex-1" style={{ color: "var(--text-primary)" }}>
                {t.message}
              </span>
              <button
                onClick={() => remove(t.id)}
                className="p-1 rounded-md transition-colors hover:bg-[var(--bg-hover)] text-[var(--text-dim)] hover:text-[var(--text-primary)] shrink-0 cursor-pointer"
              >
                <X size={14} />
              </button>
            </div>
          );
        })}
      </div>
    </Ctx.Provider>
  );
}
