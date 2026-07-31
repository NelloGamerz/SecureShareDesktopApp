import { motion } from "framer-motion";
import {
  Building2,
  Check,
  Globe,
  Mail,
  // MapPin,
  // Phone,
  Pencil,
  Plus,
  Trash2,
  UserPlus,
  Users,
  // X,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
// import { toast } from "sonner";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
// import {
//   // Table,
//   // TableBody,
//   TableCell,
//   // TableHead,
//   // TableHeader,
//   TableRow,
// } from "@/components/ui/table";
import { PageContainer, PageHeader } from "@/components/layout/page-header";
import {
  useInvitations,
  useOrganizationOverview,
  // useRespondToInvitation,
} from "./organization-hooks";
import {
  DeleteOrgDialog,
  EditOrgDialog,
  InviteMemberDialog,
} from "./organization-dialogs";
import type {
  // Invitation,
  // InvitationStatus,
  MemberRole,
} from "./organization-api";
import { useCanAccessBilling, useCurrentUserProfile } from "../auth/auth-hooks";
// import { formatRelativeTime } from "@/lib/format";

const roleVariant: Record<MemberRole, "default" | "secondary"> = {
  OWNER: "default",
  ADMIN: "secondary",
  MEMBER: "default",
};

// const inviteStatusVariant: Record<
//   InvitationStatus,
//   "default" | "secondary" | "success" | "destructive"
// > = {
//   pending: "secondary",
//   PENDING: "secondary",
//   accepted: "success",
//   rejected: "destructive",
// };

function StatCard({
  icon: Icon,
  label,
  value,
  index,
}: {
  icon: LucideIcon;
  label: string;
  value: number;
  index: number;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.05 }}
    >
      <Card>
        <CardContent className="p-5">
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-muted">
            <Icon className="h-[1.15rem] w-[1.15rem] text-muted-foreground" />
          </div>
          <p className="mt-4 text-2xl font-semibold tracking-tight">{value}</p>
          <p className="mt-0.5 text-sm text-muted-foreground">{label}</p>
        </CardContent>
      </Card>
    </motion.div>
  );
}

export function OrganizationPage() {
  const { data, isLoading, isError, error, refetch } =
    useOrganizationOverview();
  const { data: invitations = [] } = useInvitations();
  // const respondMutation = useRespondToInvitation();

  const [editOpen, setEditOpen] = useState(false);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

  const org = data?.organization ?? null;
  const members = data?.members ?? [];
  // const invitations = data?.invitations ?? [];
  const stats = data?.stats ?? null;

  const { data: profile, isLoading: profileLoading } = useCurrentUserProfile();
  const { canAccessBilling } = useCanAccessBilling();
  const navigate = useNavigate();

  useEffect(() => {
    if (!profileLoading && profile?.onboardingCompleted === false) {
      navigate("/onboarding", { replace: true });
    }
  }, [profile, profileLoading, navigate]);

  // const handleAccept = (id: string, email: string) => {
  //   acceptMutation.mutate(id, {
  //     onSuccess: () => toast.success(`${email} accepted the invitation.`),
  //     onError: (err) => toast.error(err.message),
  //   });
  // };

  // const handleReject = (id: string, email: string) => {
  //   rejectMutation.mutate(id, {
  //     onSuccess: () => toast.success(`Invitation to ${email} rejected.`),
  //     onError: (err) => toast.error(err.message),
  //   });
  // };

  // const handleAccept = (id: string, email: string) => {
  //   respondMutation.mutate(
  //     {
  //       id,
  //       action: "ACCEPT",
  //     },
  //     {
  //       onSuccess: () => {
  //         toast.success(`${email} accepted the invitation.`);
  //       },
  //       onError: (err) => {
  //         toast.error(err.message);
  //       },
  //     },
  //   );
  // };

  // const handleReject = (id: string, email: string) => {
  //   respondMutation.mutate(
  //     {
  //       id,
  //       action: "REJECT",
  //     },
  //     {
  //       onSuccess: () => {
  //         toast.success(`Invitation to ${email} rejected.`);
  //       },
  //       onError: (err) => {
  //         toast.error(err.message);
  //       },
  //     },
  //   );
  // };

  const pendingInvitesCount = invitations.filter(
    (inv) => inv.status === "PENDING",
  ).length;

  if (isLoading) {
    return (
      <PageContainer>
        <div className="space-y-2 pb-6">
          <Skeleton className="h-8 w-48" />
          <Skeleton className="h-4 w-64" />
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="p-5">
                <Skeleton className="h-9 w-9 rounded-lg" />
                <Skeleton className="mt-4 h-8 w-20" />
                <Skeleton className="mt-2 h-4 w-28" />
              </CardContent>
            </Card>
          ))}
        </div>
        <div className="mt-4 grid gap-4 lg:grid-cols-3">
          <Card className="lg:col-span-2">
            <CardHeader>
              <Skeleton className="h-5 w-40" />
            </CardHeader>
            <CardContent className="space-y-3">
              {Array.from({ length: 4 }).map((_, i) => (
                <Skeleton key={i} className="h-12 w-full" />
              ))}
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <Skeleton className="h-5 w-32" />
            </CardHeader>
            <CardContent className="space-y-3">
              {Array.from({ length: 3 }).map((_, i) => (
                <Skeleton key={i} className="h-16 w-full" />
              ))}
            </CardContent>
          </Card>
        </div>
      </PageContainer>
    );
  }

  if (isError) {
    return (
      <PageContainer>
        <PageHeader
          title="Organization"
          description="Manage your organization's profile and members."
        />
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <p className="text-sm text-muted-foreground">
              {error instanceof Error
                ? error.message
                : "Failed to load organization data."}
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

  if (!org) {
    return (
      <PageContainer>
        <PageHeader
          title="Organization"
          description="Create your organization to get started."
        />
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-full bg-muted">
              <Building2 className="h-7 w-7 text-muted-foreground" />
            </div>
            <h3 className="mt-4 text-base font-semibold">
              No organization yet
            </h3>
            <p className="mt-1 max-w-sm text-sm text-muted-foreground">
              Create your first organization to invite members and manage your
              workspace.
            </p>
            <Button asChild size="sm" className="mt-5">
              <Link to="/organization/new">
                <Plus className="h-4 w-4" />
                Create organization
              </Link>
            </Button>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  return (
    <PageContainer>
      <PageHeader
        title="Organization"
        description="Manage your organization's profile, members, and invitations."
        actions={
          <>
            {canAccessBilling && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => setInviteOpen(true)}
              >
                <UserPlus className="h-4 w-4" />
                Invite member
              </Button>
            )}
            {canAccessBilling && (
              <Button size="sm" onClick={() => setEditOpen(true)}>
                <Pencil className="h-4 w-4" />
                Edit
              </Button>
            )}
          </>
        }
      />

      {/* Stats */}
      {stats && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <StatCard
            icon={Users}
            label="Total members"
            value={stats.totalMembers}
            index={0}
          />
          <StatCard
            icon={Check}
            label="Active members"
            value={stats.activeMembers}
            index={1}
          />
          <StatCard
            icon={Mail}
            label="Pending invitations"
            value={pendingInvitesCount}
            index={2}
          />
          <StatCard
            icon={Building2}
            label="Owners & admins"
            value={stats.owners + stats.admins}
            index={3}
          />
        </div>
      )}

      <div className="mt-4 grid gap-6 lg:grid-cols-3">
        {/* Organization profile */}
        <div className="space-y-6 lg:col-span-2">
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Profile</CardTitle>
              <CardDescription>
                Organization details and contact information.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center gap-4">
                <div className="flex items-center gap-4">
                  <Avatar className="h-14 w-14 rounded-2xl">
                    <AvatarFallback className="rounded-2xl bg-primary text-primary-foreground">
                      <Building2 className="h-7 w-7" />
                    </AvatarFallback>
                  </Avatar>

                  <div>
                    <h2 className="text-lg font-semibold">{org.name}</h2>
                    <Badge variant="secondary" className="mt-1 capitalize">
                      {org.type.toLowerCase()}
                    </Badge>
                  </div>
                </div>
              </div>

              {org.type === "ORGANIZATION" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <DetailRow
                    icon={Building2}
                    label="Organization"
                    value={org.name}
                  />

                  <DetailRow
                    icon={Globe}
                    label="Industry"
                    value={org.industry}
                  />

                  <DetailRow
                    icon={Users}
                    label="Team size"
                    value={org.team_size?.toString()}
                  />

                  {/* Uncomment if available */}
                  {/* <DetailRow
      icon={MapPin}
      label="Location"
      value={[org.city, org.country].filter(Boolean).join(", ")}
    />

    <DetailRow
      icon={Phone}
      label="Phone"
      value={org.contact_phone}
    />

    <DetailRow
      icon={Mail}
      label="Contact email"
      value={org.contact_email}
    /> */}
                </div>
              )}
            </CardContent>
          </Card>

          {/* Members preview */}
          <Card>
            <CardHeader className="flex-row items-center justify-between space-y-0">
              <div>
                <CardTitle className="text-base">Members</CardTitle>
                <CardDescription>
                  {members.length} people in this organization
                </CardDescription>
              </div>
              <Button variant="ghost" size="sm" asChild>
                <Link to="/members">View all</Link>
              </Button>
            </CardHeader>
            <CardContent className="p-0">
              <div className="divide-y">
                {members.slice(0, 5).map((m) => (
                  <div key={m.id} className="flex items-center gap-3 px-6 py-3">
                    <Avatar className="h-9 w-9">
                      <AvatarFallback className="bg-muted text-xs font-medium">
                        {m.name
                          .split(" ")
                          .map((p) => p[0])
                          .join("")
                          .slice(0, 2)
                          .toUpperCase()}
                      </AvatarFallback>
                    </Avatar>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">{m.name}</p>
                      <p className="truncate text-xs text-muted-foreground">
                        {m.email}
                      </p>
                    </div>
                    <Badge
                      variant={roleVariant[m.role] ?? "default"}
                      className="capitalize"
                    >
                      {m.role}
                    </Badge>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        </div>

        {/* Invitation history */}
        <div className="space-y-6">
          {/* <Card>
            <CardHeader>
              <CardTitle className="text-base">Invitation history</CardTitle>
              <CardDescription>
                Track sent invitations and their status.
              </CardDescription>
            </CardHeader>
            <CardContent className="p-0">
              {invitationsLoading ? (
                <div className="space-y-2 p-6">
                  <Skeleton className="h-10 w-full" />
                  <Skeleton className="h-10 w-full" />
                  <Skeleton className="h-10 w-full" />
                </div>
              ) : invitations.length === 0 ? (
                <p className="px-6 py-8 text-center text-sm text-muted-foreground">
                  No invitations sent yet.
                </p>
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Email</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead className="w-20">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {invitations.map((inv) => (
                      <InvitationRow
                        key={inv.id}
                        invitation={inv}
                        onAccept={() => handleAccept(inv.id, inv.email)}
                        onReject={() => handleReject(inv.id, inv.email)}
                        acceptLoading={respondMutation.isPending}
                        rejectLoading={respondMutation.isPending}
                      />
                    ))}
                  </TableBody>
                </Table>
              )}
            </CardContent>
          </Card> */}

          {/* Danger zone */}
          {canAccessBilling && (
            <Card className="border-destructive/30">
              <CardHeader>
                <CardTitle className="text-base text-destructive">
                  Danger zone
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-xs text-muted-foreground">
                  Deleting your organization is permanent and removes all members
                  and invitations.
                </p>
                <Button
                  variant="destructive"
                  size="sm"
                  className="mt-3"
                  onClick={() => setDeleteOpen(true)}
                >
                  <Trash2 className="h-4 w-4" />
                  Delete organization
                </Button>
              </CardContent>
            </Card>
          )}
        </div>
      </div>

      {/* Dialogs */}
      <EditOrgDialog
        open={editOpen}
        onOpenChange={setEditOpen}
        organization={org}
      />
      <InviteMemberDialog
        open={inviteOpen}
        onOpenChange={setInviteOpen}
        organizationId={org.id}
      />
      <DeleteOrgDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        organization={org}
      />
    </PageContainer>
  );
}

function DetailRow({
  icon: Icon,
  label,
  value,
}: {
  icon: LucideIcon;
  label: string;
  value?: string | null;
}) {
  return (
    <div className="flex items-start gap-3">
      <Icon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0">
        <p className="text-xs text-muted-foreground">{label}</p>
        <p className="truncate text-sm font-medium">{value || "—"}</p>
      </div>
    </div>
  );
}

// function InvitationRow({
//   invitation,
//   onAccept,
//   onReject,
//   acceptLoading,
//   rejectLoading,
// }: {
//   invitation: Invitation;
//   onAccept: () => void;
//   onReject: () => void;
//   acceptLoading: boolean;
//   rejectLoading: boolean;
// }) {
//   return (
//     <TableRow>
//       <TableCell>
//         <div className="min-w-0">
//           <p className="truncate text-sm font-medium">{invitation.email}</p>
//           <p className="text-xs text-muted-foreground">
//             {invitation.role} · {formatRelativeTime(invitation.invitedAt)}
//           </p>
//         </div>
//       </TableCell>
//       <TableCell>
//         <Badge
//           variant={inviteStatusVariant[invitation.status]}
//           className="capitalize"
//         >
//           {invitation.status}
//         </Badge>
//       </TableCell>
//       <TableCell>
//         {invitation.status === "PENDING" && (
//           <div className="flex items-center gap-1">
//             <Button
//               variant="ghost"
//               size="icon"
//               className="h-7 w-7 text-success"
//               onClick={onAccept}
//               disabled={acceptLoading}
//               title="Accept"
//             >
//               <Check className="h-3.5 w-3.5" />
//             </Button>
//             <Button
//               variant="ghost"
//               size="icon"
//               className="h-7 w-7 text-destructive"
//               onClick={onReject}
//               disabled={rejectLoading}
//               title="Reject"
//             >
//               <X className="h-3.5 w-3.5" />
//             </Button>
//           </div>
//         )}
//       </TableCell>
//     </TableRow>
//   );
// }
