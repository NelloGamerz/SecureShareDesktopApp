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
  token: string;
  userId?: string;
  isAuthenticated: boolean;
}

export type ConnectionStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'error';

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
