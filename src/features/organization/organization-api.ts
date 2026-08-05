import { api } from "@/lib/api";
import { OrganizationType } from "../onboarding/onboarding-types";

// --- Types ---

export type MemberRole = "OWNER" | "ADMIN" | "MEMBER";
export type MemberStatus =
  | "active"
  | "invited"
  | "suspended"
  | "ACTIVE"
  | "INVITED"
  | "SUSPENDED";
export type InvitationStatus = "pending" | "PENDING" | "accepted" | "rejected";
export type InvitationType = "SENT" | "RECEIVED";

export interface Organization {
  id: string;
  name: string;
  website: string | null;
  industry: string | null;
  type: OrganizationType;
  team_size: number;
  contact_name: string | null;
  contact_email: string | null;
  contact_phone: string | null;
  city: string | null;
  country: string | null;
  status: MemberStatus;
  last_active_at: string;
}

export interface MemberDevice {
  id: string;
  deviceName: string | null;
  deviceIdentifier: string;
  type: string | null;
  status: string | null;
  operatingSystem: string | null;
  appVersion: string | null;
  createdAt: string | null;
}

export interface Member {
  id: string;
  name: string;
  email: string;
  role: MemberRole;
  status: MemberStatus;
  organizationId: string | null;
  devices: MemberDevice[];
  lastActiveAt: string | null;
  invitedAt: string | null;
}

export interface Invitation {
  id: string;
  organizationId: string;
  organizationName: string;
  organizationIndustry: String;
  email: string;
  role: MemberRole;
  status: InvitationStatus;
  type: InvitationType;
  invitedBy: string | null;
  invitedAt: string;
  respondedAt: string | null;
}

export interface OrgStats {
  totalMembers: number;
  activeMembers: number;
  pendingInvites: number;
  owners: number;
  admins: number;
  totalDevices: number;
  joinedAt: string;
}

export interface OrganizationOverview {
  organization: Organization | null;
  members: Member[];
  invitations: Invitation[];
  stats: OrgStats | null;
}

// --- API functions ---

export async function fetchOrganizationOverview(): Promise<OrganizationOverview> {
  const { data } = await api.get<OrganizationOverview>("/organization/me");
  return data;
}

export async function fetchMembers(): Promise<Member[]> {
  const { data } = await api.get<Member[]>("/members");
  return data;
}

export async function fetchInvitations(): Promise<Invitation[]> {
  const { data } = await api.get<Invitation[]>("/organization/invitations");
  return data;
}

export interface CreateOrgInput {
  name: string;
  website?: string;
  industry?: string;
  teamSize?: number;
  contactName?: string;
  contactEmail?: string;
  contactPhone?: string;
  city?: string;
  country?: string;
}

export async function createOrganization(
  input: CreateOrgInput,
): Promise<Organization> {
  const { data } = await api.post<Organization>("/organization", input);
  return data;
}

export type UpdateOrgInput = CreateOrgInput;

export async function updateOrganization(
  id: string,
  input: UpdateOrgInput,
): Promise<Organization> {
  const { data } = await api.put<Organization>(`/organization/${id}`, input);
  return data;
}

export async function deleteOrganization(id: string): Promise<void> {
  await api.delete(`/organization/${id}`);
}

export interface InviteMemberInput {
  organizationId?: string;
  email: string;
  role?: MemberRole;
  invitedBy?: string;
}

export async function inviteMember(
  input: InviteMemberInput,
): Promise<Invitation> {
  const { data } = await api.post<Invitation>(
    "/organization/invitation",
    input,
  );
  return data;
}

// export async function acceptInvitation(
//   id: string,
// ): Promise<{ invitation: Invitation; member: Member }> {
//   const { data } = await api.post(`/organization/invitations/${id}/accept`);
//   return data;
// }

// export async function rejectInvitation(id: string): Promise<Invitation> {
//   const { data } = await api.post<Invitation>(
//     `/organization/invitations/${id}/reject`,
//   );
//   return data;
// }

export async function changeMemberRole(
  memberId: string,
  role: MemberRole,
): Promise<Member> {
  const { data } = await api.put<Member>(
    `/organization/members/${memberId}/role`,
    { role },
  );
  return data;
}

export async function removeMember(memberId: string): Promise<void> {
  await api.delete(`/organization/members/${memberId}`);
}

export async function cancelInvitation(invitationId: string) {
  const response = await api.delete(
    `/organization/invitations/${invitationId}`,
  );

  return response.data;
}

export async function respondToInvitation(
  id: string,
  action: "ACCEPT" | "REJECT",
) {
  const { data } = await api.post(`/organization/invitations/${id}/response`, {
    action,
  });

  return data;
}
