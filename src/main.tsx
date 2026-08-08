import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/ui/ThemeProvider";
import App from "./App";
import TrayCard from "@/components/tray/TrayCard";
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

const isTrayWindow = new URLSearchParams(window.location.search).get("view") === "tray";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        {isTrayWindow ? <TrayCard /> : <App />}
      </ThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
