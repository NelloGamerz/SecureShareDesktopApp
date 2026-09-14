/**
 * Desktop OAuth configuration.
 *
 * Clerk is the only identity provider and token issuer. The desktop app is a
 * **public client**: it must never hold a Clerk secret key. PKCE replaces the
 * client secret, and the authorization code is exchanged inside the Rust layer.
 *
 * All values are `VITE_`-prefixed so they are available in the renderer; none of
 * them are secret. The client ID is public by definition for an OAuth public
 * client.
 */

const rawPublishableKey = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY ?? '';

/** Scheme registered in `src-tauri/tauri.conf.json` under `plugins.deep-link`. */
export const DESKTOP_AUTH_REDIRECT_URI =
  import.meta.env.VITE_CLERK_OAUTH_REDIRECT_URI ?? 'vilsend://auth/callback';

/**
 * Derives the Clerk Frontend API origin from a publishable key.
 *
 * A publishable key is `pk_test_`/`pk_live_` followed by the base64 encoding of
 * the instance's frontend API host, sometimes with a trailing `$`.
 * e.g. `pk_live_Y2xlcmsudmlsc2VuZC5pbiQ` → `https://clerk.vilsend.in`.
 */
export function deriveIssuerFromPublishableKey(key: string): string | null {
  const encoded = key.replace(/^pk_(test|live)_/, '');

  if (!encoded || encoded === key) {
    return null;
  }

  try {
    // atob is available in the webview; the key is base64, not base64url.
    const decoded = atob(encoded).replace(/\$$/, '').trim();

    if (!decoded || !decoded.includes('.')) {
      return null;
    }

    return `https://${decoded}`;
  } catch {
    return null;
  }
}

/**
 * Explicit override wins, otherwise the issuer is derived from the publishable
 * key so the two cannot drift apart.
 */
export const clerkIssuer =
  import.meta.env.VITE_CLERK_OAUTH_ISSUER ??
  deriveIssuerFromPublishableKey(rawPublishableKey) ??
  '';

export const clerkOAuthClientId =
  import.meta.env.VITE_CLERK_OAUTH_CLIENT_ID ?? '';

export const clerkOAuthScopes =
  import.meta.env.VITE_CLERK_OAUTH_SCOPES ?? 'openid profile email';

/**
 * Optional OIDC `prompt` sent only for the sign-up action. Clerk's hosted
 * authorization page handles both registration and sign-in; set this only if
 * the Clerk instance supports steering it toward registration.
 */
export const clerkOAuthSignUpPrompt =
  import.meta.env.VITE_CLERK_OAUTH_SIGNUP_PROMPT ?? '';

/**
 * The desktop OAuth flow needs an issuer and a client ID. Without both, the app
 * falls back to preview mode so the UI stays navigable without a real identity
 * provider.
 */
export const isDesktopAuthConfigured = Boolean(
  clerkIssuer && clerkOAuthClientId,
);

/** Payload handed to the Rust `start_desktop_auth` command. */
export interface DesktopAuthConfig {
  clientId: string;
  issuer: string;
  redirectUri: string;
  scopes: string;
  signupPrompt?: string;
}

export function getDesktopAuthConfig(): DesktopAuthConfig {
  return {
    clientId: clerkOAuthClientId,
    issuer: clerkIssuer,
    redirectUri: DESKTOP_AUTH_REDIRECT_URI,
    scopes: clerkOAuthScopes,
    ...(clerkOAuthSignUpPrompt ? { signupPrompt: clerkOAuthSignUpPrompt } : {}),
  };
}
