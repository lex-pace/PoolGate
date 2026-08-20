import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/ui/ThemeProvider";
import { GlassSettingsProvider } from "@/components/ui/GlassSettings";
import { AccountDisplayProvider } from "@/components/ui/AccountDisplay";
import { ToastProvider } from "@/components/ui/Toast";
import App from "./App";
import TrayCard from "@/components/tray/TrayCard";
import TokenMonitorTrayCard from "@/components/tray/TokenMonitorTrayCard";
import TokenMonitorAlerts from "@/components/token-monitor/TokenMonitorAlerts";
import "./styles/globals.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

const params = new URLSearchParams(window.location.search);
const isTrayWindow = params.get("view") === "tray";
const isTokenMonitorTray = params.get("view") === "token-monitor";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <GlassSettingsProvider>
        <AccountDisplayProvider>
        {isTokenMonitorTray || isTrayWindow ? (
          // 托盘窗口：主窗口的 App 自带 ToastProvider；这里为托盘补充 Toast + 告警桥接
          <ToastProvider>
            {isTokenMonitorTray ? <TokenMonitorTrayCard /> : <TrayCard />}
            <TokenMonitorAlerts />
          </ToastProvider>
        ) : (
          <App />
        )}
        </AccountDisplayProvider>
        </GlassSettingsProvider>
      </ThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
