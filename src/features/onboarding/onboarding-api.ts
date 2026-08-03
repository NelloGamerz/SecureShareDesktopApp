import { api } from '@/lib/api';
import type {
  OnboardingRequest,
  OnboardingResponse,
  // UserProfile,
} from './onboarding-types';

/** POST /onboarding — complete onboarding and create a workspace */
export async function completeOnboarding(
  payload: OnboardingRequest
): Promise<OnboardingResponse> {
  const { data } = await api.post<OnboardingResponse>('/onboarding', payload);
  return data;
}
