import { api } from '@/lib/api';
import type { UserProfile } from './auth-types';

/** GET /users/me — current user's profile + onboarding state. */
export async function fetchCurrentUser(): Promise<UserProfile> {
  const { data } = await api.get<UserProfile>('/auth/me');
  return data;
}
