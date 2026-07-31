import { motion } from "framer-motion";
import {
  Check,
  ChevronDown,
  MoreHorizontal,
  Search,
  Shield,
  Trash2,
  UserCog,
  UserPlus,
  Users,
  X,
} from "lucide-react";
import { Fragment, useState } from "react";
import { toast } from "sonner";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { PageContainer, PageHeader } from "@/components/layout/page-header";
import {
  useCancelInvitation,
  useChangeMemberRole,
  useInvitations,
  useMembers,
  useOrganizationOverview,
  useRespondToInvitation,
} from "@/features/organization/organization-hooks";
import {
  ChangeRoleDialog,
  InviteMemberDialog,
  RemoveMemberDialog,
} from "@/features/organization/organization-dialogs";
import type {
  Member,
  MemberRole,
  MemberStatus,
} from "@/features/organization/organization-api";
import { useCanAccessBilling, useCurrentUserProfile } from "@/features/auth/auth-hooks";
import { formatRelativeTime } from "@/lib/format";

const roleVariant: Record<MemberRole, "default" | "secondary"> = {
  OWNER: "default",
  ADMIN: "secondary",
  MEMBER: "default",
};

const statusVariant: Record<
  MemberStatus,
  "success" | "secondary" | "destructive"
> = {
  active: "success",
  invited: "secondary",
  suspended: "destructive",
  ACTIVE: "success",
  INVITED: "secondary",
  SUSPENDED: "destructive",
};

function initials(name: string): string {
  return name
    .split(" ")
    .map((p) => p[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

export function MembersPage() {
  const { data: overview } = useOrganizationOverview();
  const { data: members, isLoading, isError, error, refetch } = useMembers();
  const changeRoleMutation = useChangeMemberRole();
  const { canAccessBilling } = useCanAccessBilling();
  const { data: profile } = useCurrentUserProfile();

  const [query, setQuery] = useState("");
  const [expandedMemberId, setExpandedMemberId] = useState<string | null>(null);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [roleMember, setRoleMember] = useState<Member | null>(null);
  const [removeMember, setRemoveMember] = useState<Member | null>(null);
  // const acceptMutation = useAcceptInvitation();
  // const rejectMutation = useRejectInvitation();
  const respondMutation = useRespondToInvitation();
  const cancelInvitationMutation = useCancelInvitation();

  const { data: invitations = [] } =
    useInvitations();

  const pendingSentInvitations = invitations.filter(
    (inv) => inv.type === "SENT" && inv.status === "PENDING",
  );

  const pendingReceivedInvitations = invitations.filter(
    (inv) => inv.type === "RECEIVED" && inv.status === "PENDING",
  );

  const orgId = overview?.organization?.id ?? null;
  const allMembers = members ?? [];
  const canViewMemberDevices = profile?.memberRole === "OWNER";
  const filtered = allMembers.filter(
    (m) =>
      m.name.toLowerCase().includes(query.toLowerCase()) ||
      m.email.toLowerCase().includes(query.toLowerCase()),
  );

  // const handleCancelInvitation = (id: string) => {
  //   // call your cancel invitation mutation here
  //   toast.success("Invitation cancelled");
  // };

  const handleCancelInvitation = (id: string, email: string) => {
    cancelInvitationMutation.mutate(id, {
      onSuccess: () => {
        toast.success(`Invitation to ${email} cancelled.`);
      },
      onError: (err) => {
        toast.error(err.message);
      },
    });
  };

  const handleQuickRoleChange = (member: Member, role: MemberRole) => {
    changeRoleMutation.mutate(
      { memberId: member.id, role },
      {
        onSuccess: () =>
          toast.success(`${member.name}'s role updated to ${role}.`),
        onError: (err) => toast.error(err.message),
      },
    );
  };

  const toggleMemberDevices = (member: Member) => {
    if (!canViewMemberDevices) {
      return;
    }

    setExpandedMemberId((current) => (current === member.id ? null : member.id));
  };

  if (isLoading) {
    return (
      <PageContainer>
        <div className="space-y-2 pb-6">
          <Skeleton className="h-8 w-32" />
          <Skeleton className="h-4 w-56" />
        </div>
        <Card>
          <CardContent className="p-4">
            <Skeleton className="mb-3 h-9 w-56" />
            <div className="space-y-2">
              {Array.from({ length: 6 }).map((_, i) => (
                <Skeleton key={i} className="h-14 w-full" />
              ))}
            </div>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  if (isError) {
    return (
      <PageContainer>
        <PageHeader
          title="Members"
          description="Invite teammates, manage roles, and review access."
        />
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <p className="text-sm text-muted-foreground">
              {error instanceof Error
                ? error.message
                : "Failed to load members."}
            </p>
            <Button
              variant="outline"
              size="sm"
              className="mt-4"
              onClick={() => refetch()}
            >
              Try again
            </Button>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  const handleAccept = (id: string, email: string) => {
    respondMutation.mutate(
      {
        id,
        action: "ACCEPT",
      },
      {
        onSuccess: () => {
          toast.success(`Invitation from ${email} accepted.`);
        },
        onError: (err) => {
          toast.error(err.message);
        },
      },
    );
  };

  const handleReject = (id: string, email: string) => {
    respondMutation.mutate(
      {
        id,
        action: "REJECT",
      },
      {
        onSuccess: () => {
          toast.success(`Invitation from ${email} rejected.`);
        },
        onError: (err) => {
          toast.error(err.message);
        },
      },
    );
  };

  return (
    <PageContainer>
      <PageHeader
        title="Members"
        description="Invite teammates, manage roles, and review access."
        actions={
          canAccessBilling ? (
            <Button size="sm" onClick={() => setInviteOpen(true)}>
              <UserPlus className="h-4 w-4" />
              Invite member
            </Button>
          ) : null
        }
      />

      <Card>
        <CardContent className="p-4">
          <div className="relative mb-3 max-w-xs">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search members…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="pl-9"
            />
          </div>

          <div className="rounded-lg border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Member</TableHead>
                  <TableHead className="hidden md:table-cell">Role</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="hidden lg:table-cell">
                    Last active
                  </TableHead>
                  <TableHead className="w-10" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map((m, i) => (
                  <Fragment key={m.id}>
                    <motion.tr
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      transition={{ delay: i * 0.03 }}
                      className={canViewMemberDevices ? "group cursor-pointer" : "group"}
                      onClick={() => toggleMemberDevices(m)}
                    >
                      <TableCell>
                        <div className="flex items-center gap-3">
                          {canViewMemberDevices && (
                            <ChevronDown
                              className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${expandedMemberId === m.id ? "rotate-180" : ""}`}
                            />
                          )}
                          <Avatar className="h-9 w-9">
                            <AvatarFallback className="bg-muted text-xs font-medium">
                              {initials(m.name)}
                            </AvatarFallback>
                          </Avatar>
                          <div className="min-w-0">
                            <p className="truncate text-sm font-medium">
                              {m.name}
                            </p>
                            <p className="truncate text-xs text-muted-foreground">
                              {m.email}
                            </p>
                          </div>
                        </div>
                      </TableCell>
                      <TableCell className="hidden md:table-cell">
                        <div className="flex items-center gap-2">
                          <Badge
                            variant={roleVariant[m.role] ?? "default"}
                            className="capitalize"
                          >
                            {m.role}
                          </Badge>
                          {m.role === "OWNER" && (
                            <Shield className="h-3 w-3 text-muted-foreground" />
                          )}
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge
                          variant={statusVariant[m.status] ?? "default"}
                          className="capitalize"
                        >
                          {m.status}
                        </Badge>
                      </TableCell>
                      <TableCell className="hidden lg:table-cell text-sm text-muted-foreground">
                        {m.lastActiveAt
                          ? formatRelativeTime(m.lastActiveAt)
                          : "Never"}
                      </TableCell>
                      <TableCell onClick={(event) => event.stopPropagation()}>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 opacity-0 transition-opacity group-hover:opacity-100"
                            >
                              <MoreHorizontal className="h-4 w-4" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end" className="w-44">
                            <DropdownMenuItem onClick={() => setRoleMember(m)}>
                              <UserCog className="h-4 w-4" />
                              Change role
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              onClick={() => handleQuickRoleChange(m, "ADMIN")}
                              disabled={m.role === "ADMIN"}
                            >
                              Make admin
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              onClick={() => handleQuickRoleChange(m, "MEMBER")}
                              disabled={m.role === "MEMBER"}
                            >
                              Make member
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              className="text-destructive focus:text-destructive"
                              onClick={() => setRemoveMember(m)}
                              disabled={m.role === "OWNER"}
                            >
                              <Trash2 className="h-4 w-4" />
                              Remove member
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </motion.tr>
                    {canViewMemberDevices && expandedMemberId === m.id && (
                      <TableRow>
                        <TableCell colSpan={5} className="bg-muted/30">
                          <div className="rounded-md border border-dashed bg-background/70 p-3">
                            <div className="mb-2 flex items-center justify-between">
                              <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                Devices
                              </p>
                              <Badge variant="secondary" className="text-[10px]">
                                {m.devices?.length ?? 0}
                              </Badge>
                            </div>
                            {m.devices?.length ? (
                              <div className="space-y-2">
                                {m.devices.map((device) => (
                                  <div
                                    key={device.id}
                                    className="flex items-center justify-between rounded-md border px-3 py-2"
                                  >
                                    <div>
                                      <p className="text-sm font-medium">
                                        {device.deviceName ?? "Unnamed device"}
                                      </p>
                                      <p className="text-xs text-muted-foreground">
                                        {device.deviceIdentifier}
                                      </p>
                                    </div>
                                    <Badge variant="outline" className="capitalize">
                                      {device.operatingSystem ?? "Unknown"}
                                    </Badge>
                                  </div>
                                ))}
                              </div>
                            ) : (
                              <p className="text-sm text-muted-foreground">
                                No devices linked to this member yet.
                              </p>
                            )}
                          </div>
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                ))}
                {filtered.length === 0 && (
                  <TableRow>
                    <TableCell
                      colSpan={5}
                      className="py-10 text-center text-sm text-muted-foreground"
                    >
                      {query
                        ? `No members match "${query}".`
                        : "No members yet. Invite your first teammate."}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>

          {allMembers.length > 0 && (
            <p className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
              <Users className="h-3.5 w-3.5" />
              {allMembers.length}{" "}
              {allMembers.length === 1 ? "member" : "members"} total
            </p>
          )}
        </CardContent>
      </Card>

      {(pendingSentInvitations.length > 0 ||
        pendingReceivedInvitations.length > 0) && (
        <Card className="mt-6">
          <CardContent className="p-4">
            {/* Sent invitations */}
            {pendingSentInvitations.length > 0 && (
              <div className="mb-8">
                <div className="mb-4 flex items-center justify-between">
                  <div>
                    <h3 className="text-base font-semibold">
                      Sent invitations
                    </h3>
                    <p className="text-sm text-muted-foreground">
                      Invitations sent to new members.
                    </p>
                  </div>

                  <Badge variant="secondary">
                    {pendingSentInvitations.length}
                  </Badge>
                </div>

                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Email</TableHead>
                      <TableHead>Role</TableHead>
                      <TableHead>Sent</TableHead>
                      <TableHead className="w-24 text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>

                  <TableBody>
                    {pendingSentInvitations.map((invitation) => (
                      <TableRow key={invitation.id}>
                        <TableCell>{invitation.email}</TableCell>

                        <TableCell>
                          <Badge
                            variant={roleVariant[invitation.role]}
                            className="capitalize"
                          >
                            {invitation.role}
                          </Badge>
                        </TableCell>

                        <TableCell className="text-muted-foreground">
                          {formatRelativeTime(invitation.invitedAt)}
                        </TableCell>

                        <TableCell>
                          <div className="flex justify-end">
                            {/* <Button
                              size="sm"
                              variant="destructive"
                              onClick={() =>
                                handleCancelInvitation(invitation.id)
                              }
                            >
                              <X className="mr-1 h-4 w-4" />
                              Cancel
                            </Button> */}
                            <Button
                              size="sm"
                              variant="destructive"
                              disabled={cancelInvitationMutation.isPending}
                              onClick={() =>
                                handleCancelInvitation(
                                  invitation.id,
                                  invitation.email,
                                )
                              }
                            >
                              <X className="mr-1 h-4 w-4" />
                              Cancel
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}

            {/* Received invitations */}
            {pendingReceivedInvitations.length > 0 && (
              <div>
                <div className="mb-4 flex items-center justify-between">
                  <div>
                    <h3 className="text-base font-semibold">
                      Received invitations
                    </h3>

                    <p className="text-sm text-muted-foreground">
                      Invitations waiting for your response.
                    </p>
                  </div>

                  <Badge variant="secondary">
                    {pendingReceivedInvitations.length}
                  </Badge>
                </div>

                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Organization</TableHead>
                      <TableHead>Role</TableHead>
                      <TableHead>Invited</TableHead>
                      <TableHead className="w-24 text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>

                  <TableBody>
                    {pendingReceivedInvitations.map((invitation) => (
                      <TableRow key={invitation.id}>
                        <TableCell>
                          <div>
                            <p className="font-medium">
                              {invitation.organizationName}
                            </p>

                            <p className="text-xs text-muted-foreground">
                              {invitation.organizationIndustry}
                            </p>
                          </div>
                        </TableCell>

                        <TableCell>
                          <Badge
                            variant={roleVariant[invitation.role]}
                            className="capitalize"
                          >
                            {invitation.role}
                          </Badge>
                        </TableCell>

                        <TableCell className="text-muted-foreground">
                          {formatRelativeTime(invitation.invitedAt)}
                        </TableCell>

                        <TableCell>
                          <div className="flex justify-end gap-1">
                            <Button
                              size="icon"
                              variant="ghost"
                              disabled={respondMutation.isPending}
                              onClick={() =>
                                handleAccept(invitation.id, invitation.email)
                              }
                            >
                              <Check className="h-4 w-4 text-green-600" />
                            </Button>

                            <Button
                              size="icon"
                              variant="ghost"
                              disabled={respondMutation.isPending}
                              onClick={() =>
                                handleReject(invitation.id, invitation.email)
                              }
                            >
                              <X className="h-4 w-4 text-red-600" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Dialogs */}
      <InviteMemberDialog
        open={inviteOpen}
        onOpenChange={setInviteOpen}
        organizationId={orgId}
      />
      <ChangeRoleDialog
        open={roleMember !== null}
        onOpenChange={(open) => !open && setRoleMember(null)}
        member={roleMember}
      />
      <RemoveMemberDialog
        open={removeMember !== null}
        onOpenChange={(open) => !open && setRemoveMember(null)}
        member={removeMember}
      />
    </PageContainer>
  );
}
