import { ClerkProvider } from '@clerk/clerk-react';
import type { ReactNode } from 'react';
import { env, isClerkConfigured } from '@/lib/env';
import { AxiosProvider } from '@/providers/axios-provider';
import { QueryProvider } from '@/providers/query-provider';
import { ThemeProvider } from '@/providers/theme-provider';
import { AuthProvider } from '@/contexts/auth-context';
import { WebSocketProvider } from '@/contexts/websocket-context';

/**
 * Top-level provider stack.
 *
 * Order matters: Clerk must wrap AxiosProvider (which reads Clerk tokens),
 * and ThemeProvider must wrap everything that renders themed UI.
 *
 * If a Clerk publishable key is not configured we still mount the app in a
 * "preview mode" so the UI is fully navigable without a real auth backend.
 */
export function AppProviders({ children }: { children: ReactNode }) {
  if (isClerkConfigured) {
    return (
      <ClerkProvider publishableKey={env.clerkPublishableKey}>
        <AxiosProvider>
          <QueryProvider>
            <ThemeProvider>
              <AuthProvider>
                <WebSocketProvider>{children}</WebSocketProvider>
              </AuthProvider>
            </ThemeProvider>
          </QueryProvider>
        </AxiosProvider>
      </ClerkProvider>
    );
  }

  // Preview mode: Clerk not configured — skip Clerk/Axios wiring.
  return (
    <QueryProvider>
      <ThemeProvider>
        <AuthProvider>
          <WebSocketProvider>{children}</WebSocketProvider>
        </AuthProvider>
      </ThemeProvider>
    </QueryProvider>
  );
}
