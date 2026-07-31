// import { useAuth } from '@clerk/clerk-react';
// import { useEffect, type ReactNode } from 'react';
// import { setTokenGetter } from '@/lib/api';

// /**
//  * Bridges Clerk's session token into the shared axios instance.
//  * Mount once inside <ClerkProvider>. Every axios request will then
//  * automatically carry a fresh Bearer JWT.
//  */
// export function AxiosProvider({ children }: { children: ReactNode }) {
//   const { getToken, isSignedIn } = useAuth();

//   useEffect(() => {
//     setTokenGetter(async () => {
//       if (!isSignedIn) return null;
//       try {
//         return await getToken();
//       } catch {
//         return null;
//       }
//     });
//   }, [getToken, isSignedIn]);

//   return <>{children}</>;
// }


import { useAuth } from '@clerk/clerk-react';
import { useEffect, type ReactNode } from 'react';
import { setTokenGetter } from '@/lib/api';
import { info, error } from '@tauri-apps/plugin-log';

export function AxiosProvider({ children }: { children: ReactNode }) {
  const { getToken, isSignedIn, isLoaded } = useAuth();

  useEffect(() => {
    if (!isLoaded) return;

    info(`Clerk loaded: ${isLoaded}`);
    info(`Clerk signed in: ${!!isSignedIn}`);

    setTokenGetter(async () => {
      if (!isSignedIn) {
        await info("Clerk user is not signed in");
        return null;
      }

      try {
        const token = await getToken();

        await info(`Token exists: ${!!token}`);
        await info(`Token length: ${token?.length ?? 0}`);

        return token;
      } catch (err) {
        await error(`getToken failed: ${String(err)}`);
        return null;
      }
    });
  }, [getToken, isSignedIn, isLoaded]);

  return <>{children}</>;
}