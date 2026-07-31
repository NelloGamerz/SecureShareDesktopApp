import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelInvitation,
  changeMemberRole,
  createOrganization,
  deleteOrganization,
  fetchInvitations,
  fetchMembers,
  fetchOrganizationOverview,
  respondToInvitation,
  inviteMember,
  removeMember,
  updateOrganization,
  type CreateOrgInput,
  type Invitation,
  type Member,
  type MemberRole,
  type Organization,
  type UpdateOrgInput,
} from "./organization-api";

const OVERVIEW_KEY = ["organization-overview"];
const MEMBERS_KEY = ["organization-members"];
const INVITATIONS_KEY = ["organization-invitations"];

export function useOrganizationOverview() {
  return useQuery({
    queryKey: OVERVIEW_KEY,
    queryFn: fetchOrganizationOverview,
    staleTime: 30_000,
  });
}

export function useMembers() {
  return useQuery({
    queryKey: MEMBERS_KEY,
    queryFn: fetchMembers,
    staleTime: 30_000,
  });
}

export function useInvitations() {
  return useQuery({
    queryKey: INVITATIONS_KEY,
    queryFn: fetchInvitations,
    staleTime: 30_000,
  });
}

function invalidateAll(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: OVERVIEW_KEY });
  qc.invalidateQueries({ queryKey: MEMBERS_KEY });
  qc.invalidateQueries({ queryKey: INVITATIONS_KEY });
}

export function useCreateOrganization() {
  const qc = useQueryClient();
  return useMutation<Organization, Error, CreateOrgInput>({
    mutationFn: createOrganization,
    onSuccess: () => invalidateAll(qc),
  });
}

export function useUpdateOrganization() {
  const qc = useQueryClient();
  return useMutation<
    Organization,
    Error,
    { id: string; input: UpdateOrgInput }
  >({
    mutationFn: ({ id, input }) => updateOrganization(id, input),
    onSuccess: () => invalidateAll(qc),
  });
}

export function useDeleteOrganization() {
  const qc = useQueryClient();
  return useMutation<void, Error, string>({
    mutationFn: deleteOrganization,
    onSuccess: () => invalidateAll(qc),
  });
}

export function useInviteMember() {
  const qc = useQueryClient();
  return useMutation<
    Invitation,
    Error,
    {
      organizationId: string;
      email: string;
      role: MemberRole;
      invitedBy?: string;
    }
  >({
    mutationFn: inviteMember,
    onSuccess: () => invalidateAll(qc),
  });
}

// export function useAcceptInvitation() {
//   const qc = useQueryClient();
//   return useMutation<{ invitation: Invitation; member: Member }, Error, string>(
//     {
//       mutationFn: acceptInvitation,
//       onSuccess: () => invalidateAll(qc),
//     },
//   );
// }

export function useCancelInvitation() {
  const qc = useQueryClient();

  return useMutation<Invitation, Error, string>({
    mutationFn: cancelInvitation,
    onSuccess: () => invalidateAll(qc),
  });
}

// export function useRejectInvitation() {
//   const qc = useQueryClient();
//   return useMutation<Invitation, Error, string>({
//     mutationFn: rejectInvitation,
//     onSuccess: () => invalidateAll(qc),
//   });
// }

export function useRespondToInvitation() {
  const qc = useQueryClient();

  return useMutation<
    void,
    Error,
    {
      id: string;
      action: "ACCEPT" | "REJECT";
    }
  >({
    mutationFn: ({ id, action }) => respondToInvitation(id, action),
    onSuccess: () => invalidateAll(qc),
  });
}

export function useChangeMemberRole() {
  const qc = useQueryClient();
  return useMutation<Member, Error, { memberId: string; role: MemberRole }>({
    mutationFn: ({ memberId, role }) => changeMemberRole(memberId, role),
    onSuccess: () => invalidateAll(qc),
  });
}

export function useRemoveMember() {
  const qc = useQueryClient();
  return useMutation<void, Error, string>({
    mutationFn: removeMember,
    onSuccess: () => invalidateAll(qc),
  });
}
