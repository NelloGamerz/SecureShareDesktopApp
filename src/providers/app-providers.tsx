import type { ReactNode } from "react";
import { AxiosProvider } from "@/providers/axios-provider";
import { QueryProvider } from "@/providers/query-provider";
import { ThemeProvider } from "@/providers/theme-provider";
import { AuthProvider } from "@/contexts/auth-context";
import { WebSocketProvider } from "@/contexts/websocket-context";

/**
 * Top-level provider stack.
 *
 * There is no Clerk React provider: authentication happens in the system
 * browser through the Rust layer, so the renderer never hosts a Clerk session.
 * `AxiosProvider` is independent of the session — its token getter asks the
 * Tauri layer for the current token on each request.
 */
export function AppProviders({ children }: { children: ReactNode }) {
  return (
    <AxiosProvider>
      <QueryProvider>
        <ThemeProvider>
          <AuthProvider>
            <WebSocketProvider>{children}</WebSocketProvider>
          </AuthProvider>
        </ThemeProvider>
      </QueryProvider>
    </AxiosProvider>
  );
}
