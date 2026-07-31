import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { fetchCurrentUser } from "./auth-api";
import type { UserProfile } from "./auth-types";

/** Global TanStack Query key for the current user's profile. */
export const PROFILE_KEY = ["auth", "user-profile"] as const;

/**
 * Global hook — the current user's profile + onboarding state.
 * Used by the auth guard, the onboarding page, the sidebar user menu, etc.
 */
export function useCurrentUserProfile() {
  return useQuery<UserProfile>({
    queryKey: PROFILE_KEY,
    queryFn: fetchCurrentUser,
    staleTime: 60_000,
    retry: 1,
  });
}

/**
 * Reusable permission helper for views/routes that should be hidden for
 * organization members.
 */
export function useCanAccessBilling() {
  const { data: profile, isLoading } = useCurrentUserProfile();

  const isRestrictedMember =
    profile?.organizationType === "ORGANIZATION" &&
    profile.memberRole === "MEMBER";

  return {
    canAccessBilling: !isRestrictedMember,
    isRestrictedMember,
    isLoading,
    profile,
  };
}

/**
 * Hide the members section for individual owners, since they do not have an
 * organization workspace to manage members for.
 */
export function useCanAccessMembers() {
  const { data: profile, isLoading } = useCurrentUserProfile();

  const isIndividualOwner =
    profile?.organizationType === "INDIVIDUAL" &&
    profile.memberRole === "OWNER";

  return {
    canAccessMembers: !isIndividualOwner,
    isIndividualOwner,
    isLoading,
    profile,
  };
}

/**
 * Returns a function that decides where to send the user after a successful
 * login or register. It invalidates the cached profile, refetches it, then
 * navigates to /onboarding or /dashboard based on `onboardingCompleted`.
 *
 * Usage:
 *   const redirectToApp = usePostAuthRedirect();
 *   // after successful login:
 *   await redirectToApp();
 */
export function usePostAuthRedirect() {
  const navigate = useNavigate();
  const qc = useQueryClient();

  return async (fallback = "/organization") => {
    qc.invalidateQueries({ queryKey: PROFILE_KEY });
    try {
      const profile = await fetchCurrentUser();
      console.log("POST LOGIN PROFILE:", profile);
      console.log("ONBOARDING STATUS:", profile.onboardingCompleted);
      qc.setQueryData<UserProfile>(PROFILE_KEY, profile);
      navigate(profile.onboardingCompleted ? "/organization" : "/onboarding", {
        replace: true,
      });
    } catch {
      navigate(fallback, { replace: true });
    }
  };
}
