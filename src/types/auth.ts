import { OrganizationType } from "@/features/onboarding/onboarding-types";
import { MemberRole } from "@/features/organization/organization-api";

export interface User {
  id?: string;
  name?: string;
  email?: string;
  memberRole?: MemberRole;
  organizationType?: OrganizationType;
}

export interface Session {
  /**
   * The access token is intentionally absent from the renderer.
   *
   * It lives in the Rust `AuthState` and is fetched per request by the axios
   * interceptor through the `get_auth_token` command, which also refreshes it.
   */
  token?: string;
  userId?: string;
  isAuthenticated: boolean;
}

/**
 * Mirrors `ConnectionStatus` in `src-tauri/src/models/mod.rs`.
 *
 * Serde serialises the unit variants under their Rust names, so the wire
 * values are PascalCase. `Error` carries a message and is serialised
 * externally tagged. These spellings are the ones `get_connection_status`
 * returns and the ones a `server-event` payload carries — do not lower-case
 * them, and do not compare against lower-case literals.
 */
export type ConnectionStatus =
  | 'Disconnected'
  | 'Connecting'
  | 'Connected'
  | 'Reconnecting'
  | { Error: string };

export interface WebSocketMessage {
  type: string;
  payload: unknown;
}

export interface ServerEvent {
  type: 'connection-status' | 'message' | 'error';
  payload: unknown;
}

export interface TauriAuthStatePayload {
  type: string;
  isAuthenticated: boolean;
  userId?: string;
}
