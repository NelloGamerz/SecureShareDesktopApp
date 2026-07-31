import { MemberRole } from "../organization/organization-api";

export type OrganizationType = "INDIVIDUAL" | "ORGANIZATION";

export type OrganizationSize = "1-5" | "6-20" | "21-50" | "51-100" | "100+";

export type DeviceType = "DESKTOP" | "MOBILE" | "TABLET" | "LAPTOP" | "UNKNOWN";

export type Industry =
  | "SOFTWARE"
  | "VIDEO_EDITING"
  | "MARKETING"
  | "EDUCATION"
  | "AGENCY"
  | "MANUFACTURING"
  | "OTHER";

export interface DeviceInfo {
  deviceName: string;
  deviceIdentifier: string;
  deviceType: DeviceType;
  operatingSystem: string;
  appVersion: string;
}

/** GET /onboarding/users/me response */
export interface UserProfile {
  id: string;
  clerkUserId: string;
  email: string | null;
  firstName: string | null;
  lastName: string | null;
  imageUrl: string | null;
  onboardingCompleted: boolean;
  memberRole: MemberRole;
  organizationType: OrganizationType;
}

/** POST /onboarding request — Individual */
export interface IndividualOnboardingRequest {
  organizationName: string;
  organizationType: "INDIVIDUAL";
  device: DeviceInfo;
  publicKey: string;
}

/** POST /onboarding request — Organization */
export interface OrganizationOnboardingRequest {
  organizationName: string;
  organizationType: "ORGANIZATION";
  organizationSize: OrganizationSize;
  industry: Industry;
  workspaceSlug?: string;
  invites: string[];
  device: DeviceInfo;
  publicKey: string;
}

export type OnboardingRequest =
  | IndividualOnboardingRequest
  | OrganizationOnboardingRequest;


export interface OnboardingResponse {
  tunnel_id: string;
  hostname: string;
  tunnelToken: string | null;
  newlyCreated: boolean;
}

/** Shape held in the multi-step form state (before submission). */
export interface OnboardingFormState {
  organizationType: OrganizationType | null;
  workspaceName: string;
  organizationName: string;
  organizationSize: OrganizationSize | "";
  industry: Industry | "";
  workspaceSlug: string;
  invites: string[];
  agreedToTerms: boolean;
}
