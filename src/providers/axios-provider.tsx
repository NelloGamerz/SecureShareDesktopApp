import { useEffect, type ReactNode } from 'react';
import { getAuthToken } from '@/api/tauri';
import { setTokenGetter } from '@/lib/api';

/**
 * Bridges the desktop session token into the shared axios instance.
 *
 * The token lives in the Rust `AuthState`, not in React, so this getter asks
 * the Tauri layer for it on every request. That keeps `Authorization` current
 * without a polling timer: `get_auth_token` returns the stored token and
 * refreshes it when it is close to expiry.
 */
export function AxiosProvider({ children }: { children: ReactNode }) {
  useEffect(() => {
    setTokenGetter(async () => {
      try {
        return await getAuthToken();
      } catch {
        // No session, or the refresh failed and Rust cleared it. Either way the
        // request goes out unauthenticated and a 401 is surfaced normally.
        return null;
      }
    });

    return () => setTokenGetter(() => Promise.resolve(null));
  }, []);

  return <>{children}</>;
}
