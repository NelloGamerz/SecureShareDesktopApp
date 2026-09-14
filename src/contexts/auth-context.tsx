import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  cancelDesktopAuth,
  getAuthStatus,
  logoutFromTauri,
  startDesktopAuth,
  stopCloudflared,
  stopTauriWebSocket,
} from "@/api/tauri";
import {
  getDesktopAuthConfig,
  isDesktopAuthConfigured,
} from "@/lib/auth-config";
import type { Session, User } from "@/types/auth";

/** How long to keep the "waiting for your browser" state before giving up. */
const AUTH_ATTEMPT_TIMEOUT_MS = 5 * 60 * 1000;

type AuthMode = "sign_in" | "sign_up";

interface AuthStateChangedPayload {
  type: string;
  isAuthenticated: boolean;
  userId?: string | null;
}

interface AuthErrorPayload {
  type: string;
  message?: string;
}

interface AuthContextValue {
  session: Session | null;
  user: User | null;
  isAuthenticated: boolean;
  /** False until the first status read from the Tauri layer settles. */
  isLoaded: boolean;
  /** True while the system browser holds an in-flight authorization. */
  isAuthenticating: boolean;
  /** Last sign-in failure, already safe to display. */
  error: string | null;
  /** True when no Clerk OAuth client is configured (UI-only preview mode). */
  isPreviewMode: boolean;
  signIn: () => Promise<void>;
  signUp: () => Promise<void>;
  cancelAuthentication: () => Promise<void>;
  clearError: () => void;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

/**
 * Desktop session provider.
 *
 * Clerk remains the identity provider and token issuer, but authentication now
 * happens in the **system browser** using Authorization Code + PKCE:
 *
 *   1. `signIn`/`signUp` ask Rust to mint a PKCE challenge and open the browser.
 *   2. Clerk redirects to `vilsend://auth/callback`.
 *   3. Rust validates `state`, exchanges the code, and stores the token.
 *   4. Rust emits `auth-state-changed`, which updates this provider.
 *
 * The access token deliberately never enters React state — it stays in the Rust
 * `AuthState` and is fetched per request by the axios interceptor.
 */
export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [user, setUser] = useState<User | null>(null);
  // Always start signed out. A session can only ever be granted by the Rust
  // layer, so assuming one here would let unauthenticated users into protected
  // routes and bounce them back off the sign-in screen in a loop.
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /** Guards against a second flow starting from a double-click. */
  const attemptInFlight = useRef(false);

  const applySignedIn = useCallback((userId?: string | null) => {
    setSession({ isAuthenticated: true, userId: userId ?? undefined });
    setUser({ id: userId ?? undefined });
    setIsAuthenticated(true);
    setIsAuthenticating(false);
    setError(null);
    attemptInFlight.current = false;
  }, []);

  const applySignedOut = useCallback(() => {
    setSession(null);
    setUser(null);
    setIsAuthenticated(false);
    setIsAuthenticating(false);
    attemptInFlight.current = false;
  }, []);

  /*
   * Bootstrap.
   *
   * Reads the authoritative state from Rust and subscribes to changes. The
   * `cancelled` flag plus the post-`await` check keep a React StrictMode
   * double-mount (or a fast unmount) from leaking a listener.
   */
  useEffect(() => {
    let cancelled = false;
    let unlistenState: UnlistenFn | undefined;
    let unlistenError: UnlistenFn | undefined;

    const bootstrap = async () => {
      if (!isDesktopAuthConfigured) {
        if (!cancelled) setIsLoaded(true);
        return;
      }

      try {
        const status = await getAuthStatus();
        if (cancelled) return;

        if (status.isAuthenticated) {
          applySignedIn(status.userId);
        } else {
          applySignedOut();
        }

        if (status.isAuthenticating) {
          setIsAuthenticating(true);
        }
      } catch {
        // A failed status read must not strand the app on a loading screen;
        // fall through to the unauthenticated state.
        if (!cancelled) applySignedOut();
      } finally {
        if (!cancelled) setIsLoaded(true);
      }
    };

    const subscribe = async () => {
      const stateListener = await listen<AuthStateChangedPayload>(
        "auth-state-changed",
        (event) => {
          if (cancelled) return;

          if (event.payload?.isAuthenticated) {
            applySignedIn(event.payload.userId);
          } else {
            applySignedOut();
          }

          setIsLoaded(true);
        },
      );

      if (cancelled) {
        stateListener();
        return;
      }

      unlistenState = stateListener;

      const errorListener = await listen<AuthErrorPayload>(
        "auth-error",
        (event) => {
          if (cancelled) return;

          setError(event.payload?.message ?? "Sign-in could not be completed.");
          setIsAuthenticating(false);
          attemptInFlight.current = false;
        },
      );

      if (cancelled) {
        errorListener();
        return;
      }

      unlistenError = errorListener;
    };

    void bootstrap();
    void subscribe();

    return () => {
      cancelled = true;
      unlistenState?.();
      unlistenError?.();
    };
  }, [applySignedIn, applySignedOut]);

  /*
   * Abandon the waiting state if the user never finishes in the browser, so a
   * closed tab cannot leave the button spinning forever.
   */
  useEffect(() => {
    if (!isAuthenticating) return;

    const timeout = window.setTimeout(() => {
      void cancelDesktopAuth().catch(() => undefined);
      setIsAuthenticating(false);
      attemptInFlight.current = false;
      setError(
        "Sign-in was not completed. Please try again and finish in your browser.",
      );
    }, AUTH_ATTEMPT_TIMEOUT_MS);

    return () => window.clearTimeout(timeout);
  }, [isAuthenticating]);

  const beginAuth = useCallback(
    async (mode: AuthMode) => {
      if (!isDesktopAuthConfigured) {
        setError(
          "Authentication is not configured. Set VITE_CLERK_OAUTH_CLIENT_ID and a valid Clerk publishable key.",
        );
        return;
      }

      // A second click while the browser is open would start a competing PKCE
      // flow and make the callback ambiguous.
      if (attemptInFlight.current || isAuthenticating) {
        return;
      }

      attemptInFlight.current = true;
      setError(null);
      setIsAuthenticating(true);

      try {
        await startDesktopAuth(getDesktopAuthConfig(), mode);
      } catch (err) {
        attemptInFlight.current = false;
        setIsAuthenticating(false);
        setError(
          err instanceof Error
            ? err.message
            : "Could not open your browser to sign in.",
        );
      }
    },
    [isAuthenticating],
  );

  const signIn = useCallback(() => beginAuth("sign_in"), [beginAuth]);
  const signUp = useCallback(() => beginAuth("sign_up"), [beginAuth]);

  const cancelAuthentication = useCallback(async () => {
    try {
      await cancelDesktopAuth();
    } catch {
      // Cancelling is best-effort; the local UI state is what matters here.
    } finally {
      setIsAuthenticating(false);
      attemptInFlight.current = false;
    }
  }, []);

  const clearError = useCallback(() => setError(null), []);

  const logout = useCallback(async () => {
    // Stop local services first so nothing keeps running with a dead session.
    await Promise.allSettled([stopTauriWebSocket(), stopCloudflared()]);

    // Rust clears the token, refresh token and any pending authorization, then
    // emits the signed-out state.
    await logoutFromTauri();

    applySignedOut();
  }, [applySignedOut]);

  const value = useMemo(
    () => ({
      session,
      user,
      isAuthenticated,
      isLoaded,
      isAuthenticating,
      error,
      isPreviewMode: !isDesktopAuthConfigured,
      signIn,
      signUp,
      cancelAuthentication,
      clearError,
      logout,
    }),
    [
      error,
      isAuthenticated,
      isAuthenticating,
      isLoaded,
      signIn,
      signUp,
      cancelAuthentication,
      clearError,
      logout,
      session,
      user,
    ],
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
