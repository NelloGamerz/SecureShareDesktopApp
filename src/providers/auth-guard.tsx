import type { ReactNode } from 'react';
import { Navigate } from 'react-router-dom';
import { LoadingScreen } from '@/components/layout/loading-screen';
import { useAuth } from '@/contexts/auth-context';
import { useCurrentUserProfile } from '@/features/auth/auth-hooks';

/**
 * ProtectedRoute — gates a subtree behind a desktop session.
 *
 * Waits for the first status read from the Tauri layer before deciding, so a
 * signed-in user is not bounced to `/sign-in` during startup.
 */
export function ProtectedRoute({ children }: { children: ReactNode }) {
  const { isAuthenticated, isLoaded } = useAuth();

  if (!isLoaded) {
    return <LoadingScreen label="Starting VilSend…" />;
  }

  if (!isAuthenticated) {
    return <Navigate to="/sign-in" replace />;
  }

  return <>{children}</>;
}

/** AuthGate — keeps already-authenticated users off the sign-in/sign-up pages. */
export function AuthGate({ children }: { children: ReactNode }) {
  const { isAuthenticated, isLoaded } = useAuth();

  if (!isLoaded) {
    return <LoadingScreen label="Starting VilSend…" />;
  }

  if (isAuthenticated) {
    return <Navigate to="/organization" replace />;
  }

  return <>{children}</>;
}

/**
 * Normalized user object for display.
 *
 * Clerk no longer exposes the user to the renderer, so display fields come from
 * the application's own profile endpoint plus the local session.
 */
export function useCurrentUser() {
  const { isAuthenticated, isLoaded, user } = useAuth();
  const { data: profile, isLoading } = useCurrentUserProfile({
    enabled: isAuthenticated,
  });

  const firstName = profile?.firstName ?? '';
  const name = firstName || 'User';

  return {
    isLoaded: isLoaded && !isLoading,
    isSignedIn: isAuthenticated,
    id: user?.id ?? profile?.id,
    name,
    firstName,
    // Not returned by the profile endpoint yet; kept so display components do
    // not have to special-case a missing field.
    lastName: '',
    email: '',
    imageUrl: '',
    initials: firstName ? firstName.slice(0, 2).toUpperCase() : 'U',
  } as const;
}

export { Navigate };
