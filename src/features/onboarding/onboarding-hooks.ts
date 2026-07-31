import { useMutation, useQueryClient } from '@tanstack/react-query';
import { completeOnboarding } from './onboarding-api';
import type { OnboardingRequest, OnboardingResponse } from './onboarding-types';
import { PROFILE_KEY } from '../auth/auth-hooks';
// import { fetchCurrentUser } from '../auth/auth-api';
import { UserProfile } from '../auth/auth-types';


// export function useCurrentUserProfile() {
//   return useQuery<UserProfile>({
//     queryKey: PROFILE_KEY,
//     queryFn: fetchCurrentUser,
//     staleTime: 60_000,
//     retry: 1,
//   });
// }

export function useCompleteOnboarding() {
  const qc = useQueryClient();
  return useMutation<OnboardingResponse, Error, OnboardingRequest>({
    mutationFn: completeOnboarding,
    onSuccess: (data) => {
      // Optimistically mark the profile as onboarded so the guard lets the
      // user through to /dashboard without a re-fetch.
      qc.setQueryData<UserProfile>(PROFILE_KEY, (prev) =>
        prev ? { ...prev, onboardingCompleted: true } : prev
      );
      return data;
    },
  });
}
