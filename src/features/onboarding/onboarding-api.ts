import { api } from '@/lib/api';
import type {
  OnboardingRequest,
  OnboardingResponse,
  // UserProfile,
} from './onboarding-types';



// /** GET /onboarding/users/me — current user's profile + onboarding state */
// export async function fetchCurrentUser(): Promise<UserProfile> {
//   const { data } = await api.get<UserProfile>('/auth/me');
//   return data;
// }

/** POST /onboarding — complete onboarding and create a workspace */
export async function completeOnboarding(
  payload: OnboardingRequest
): Promise<OnboardingResponse> {
  const { data } = await api.post<OnboardingResponse>('/onboarding', payload);
  return data;
}
