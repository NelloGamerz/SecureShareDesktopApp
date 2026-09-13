import { useEffect } from 'react';
import { useAuth, useClerk } from '@clerk/clerk-react';

export function ClerkDebug() {
  const {
    isLoaded,
    isSignedIn,
    sessionId,
    getToken,
  } = useAuth();

  const clerk = useClerk();

  useEffect(() => {
    if (!isLoaded) return;

    async function test() {
      try {
        const token = await getToken();

        console.log('CLERK TOKEN RESULT', {
          hasToken: !!token,
          tokenLength: token?.length,
        });
      } catch (error) {
        console.error('CLERK TOKEN ERROR', error);
      }
    }

    test();
  }, [isLoaded, getToken]);

  console.log('CLERK DEBUG', {
    origin: window.location.origin,
    isLoaded,
    isSignedIn,
    sessionId,
    isStandardBrowser: clerk.isStandardBrowser,
    instanceType: clerk.instanceType,
    domain: clerk.domain,
  });

  return null;
}