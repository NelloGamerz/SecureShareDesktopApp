import { OrganizationType } from "../onboarding/onboarding-types";
import { MemberRole } from "../organization/organization-api";

/** Global user profile — returned by GET /users/me on your backend. */
export interface UserProfile {
  id: string;
//   email: string | null;
  firstName: string | null;
//   lastName: string | null;
//   imageUrl: string | null;
  onboardingCompleted: boolean;
  memberRole?: MemberRole;
  organizationType?: OrganizationType;
}
