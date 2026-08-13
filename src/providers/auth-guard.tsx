import {
  SignedIn as ClerkSignedIn,
  SignedOut as ClerkSignedOut,
  useUser,
} from '@clerk/clerk-react';
import type { ReactNode } from 'react';
import { Navigate } from 'react-router-dom';
import { isClerkConfigured } from '@/lib/env';

/**
 * ProtectedRoute — gates a subtree behind authentication.
 * In preview mode (Clerk not configured) it renders children directly.
 */
export function ProtectedRoute({ children }: { children: ReactNode }) {
  if (!isClerkConfigured) {
    return <>{children}</>;
  }

  return (
    <ClerkSignedIn>
      {children}
    </ClerkSignedIn>
  );
}

/** AuthGate — redirects signed-in users away from auth pages. */
export function AuthGate({ children }: { children: ReactNode }) {
  if (!isClerkConfigured) {
    return <>{children}</>;
  }
  return <ClerkSignedOut>{children}</ClerkSignedOut>;
}

/** Hook that returns a normalized user object across Clerk/preview modes. */
export function useCurrentUser() {
  const { user, isLoaded, isSignedIn } = useUser();

  if (!isClerkConfigured) {
    return {
      isLoaded: true,
      isSignedIn: true,
      name: 'Demo User',
      email: 'demo@vilsend.io',
      imageUrl: '',
      initials: 'DU',
    } as const;
  }

  return {
    isLoaded,
    isSignedIn,
    name: user?.fullName ?? user?.username ?? 'User',
    email: user?.primaryEmailAddress?.emailAddress ?? '',
    imageUrl: user?.imageUrl ?? '',
    firstName: user?.firstName ?? '',
    lastName: user?.lastName ?? '',
    initials:
      (user?.firstName?.[0] ?? '') + (user?.lastName?.[0] ?? '') || 'U',
  } as const;
}

export { ClerkSignedIn, ClerkSignedOut };
export { Navigate };
