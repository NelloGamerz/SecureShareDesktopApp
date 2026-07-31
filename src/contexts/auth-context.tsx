import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { useAuth as useClerkAuth } from "@clerk/clerk-react";
import { listen } from "@tauri-apps/api/event";
import {
  loginWithTauri,
  logoutFromTauri,
  stopCloudflared,
  stopTauriWebSocket,
  updateAuthToken,
} from "@/api/tauri";
import type { Session, User } from "@/types/auth";
import { getDeviceInfo } from "@/services/getDeviceInfo";

interface AuthContextValue {
  session: Session | null;
  user: User | null;
  isAuthenticated: boolean;
  login: (token: string, userId?: string) => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const auth = useClerkAuth();
  const { getToken, isSignedIn, isLoaded } = auth;
  const clerkUser = (
    auth as typeof auth & { user?: { id?: string; fullName?: string | null } }
  ).user;
  const [session, setSession] = useState<Session | null>(null);
  const [user, setUser] = useState<User | null>(null);
  const [isAuthenticated, setIsAuthenticated] = useState(false);

  useEffect(() => {
    if (!isLoaded) return;
    if (!isSignedIn) {
      setIsAuthenticated(false);
      setSession(null);
      setUser(null);
      return;
    }

    const bootstrap = async () => {
      const token = await getToken();
      if (!token) return;
      const deviceInfo = await getDeviceInfo();
      await loginWithTauri(token, deviceInfo);
      // await startTauriWebSocket(deviceInfo);
      setSession({ token, userId: clerkUser?.id, isAuthenticated: true });
      setUser({ id: clerkUser?.id, name: clerkUser?.fullName ?? undefined, });
      setIsAuthenticated(true);
    };

    void bootstrap();
  }, [clerkUser?.id, clerkUser?.fullName, getToken, isLoaded, isSignedIn]);

  useEffect(() => {
    if (!isLoaded || !isSignedIn) return;

    let cancelled = false;

    const syncToken = async () => {
      const token = await getToken();

      if (!token || cancelled) return;

      await updateAuthToken(token);
    };

    // sync immediately
    void syncToken();

    // refresh periodically
    const interval = setInterval(() => {
      void syncToken();
    }, 30_000);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [getToken, isLoaded, isSignedIn]);

  useEffect(() => {
    let ignored = false;
    const subscribe = async () => {
      const unlisten = await listen<{
        isAuthenticated: boolean;
        userId?: string;
      }>("auth-state-changed", (event) => {
        if (ignored) return;
        setIsAuthenticated(event.payload.isAuthenticated);
        if (event.payload.isAuthenticated) {
          setSession((prev) =>
            prev
              ? { ...prev, isAuthenticated: true, userId: event.payload.userId }
              : null,
          );
        } else {
          setSession(null);
        }
      });
      return unlisten;
    };

    void subscribe().then((unlisten) => {
      if (ignored) {
        unlisten?.();
      }
    });

    return () => {
      ignored = true;
    };
  }, []);

  const login = async (token: string, userId?: string) => {
    const deviceInfo = await getDeviceInfo();
    await loginWithTauri(token, deviceInfo);
    // await startTauriWebSocket(deviceInfo);
    setSession({ token, userId, isAuthenticated: true });
    setIsAuthenticated(true);
  };

  const logout = async () => {
    await stopTauriWebSocket();
    await stopCloudflared();
    await logoutFromTauri();
    setSession(null);
    setUser(null);
    setIsAuthenticated(false);
  };

  const value = useMemo(
    () => ({ session, user, isAuthenticated, login, logout }),
    [isAuthenticated, session, user],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth must be used within AuthProvider");
  }
  return context;
}
