import { getAuthToken, logoutFromTauri } from '@/api/tauri';

/**
 * Reads the current desktop session token.
 *
 * Prefer letting the shared axios instance attach the header automatically.
 * This is exposed for the few call sites that need a token outside axios
 * (for example, constructing a WebSocket URL).
 *
 * Resolves to `null` when there is no session.
 */
export async function getSessionToken(): Promise<string | null> {
  return getAuthToken();
}

/** Ends the desktop session. Tokens are cleared in the Rust layer. */
export async function deauthenticateWithTauri() {
  await logoutFromTauri();
}
