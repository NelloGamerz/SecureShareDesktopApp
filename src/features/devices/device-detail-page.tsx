import { motion } from 'framer-motion';
import {
  // ArrowDownToLine,
  ArrowLeft,
  // ArrowUpFromLine,
  // Cpu,
  // Globe,
  HardDrive,
  Laptop,
  Pencil,
  Trash2,
  // User,
  // Wifi,
  // WifiOff,
} from 'lucide-react';
import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Avatar, AvatarFallback } from '@/components/ui/avatar';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
// import { Progress } from '@/components/ui/progress';
import { Separator } from '@/components/ui/separator';
import { Skeleton } from '@/components/ui/skeleton';
import { PageContainer } from '@/components/layout/page-header';
import { useDeviceDetail } from './devices-hooks';
import { RenameDeviceDialog, RemoveDeviceDialog } from './devices-dialogs';
import { formatRelativeTime } from '@/lib/format';
import { statusVariant, typeIcon } from './devices-page';


// function healthColor(health: number): string {
//   if (health >= 80) return 'hsl(var(--success))';
//   if (health >= 50) return 'hsl(var(--warning))';
//   return 'hsl(var(--destructive))';
// }

export function DeviceDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { data: device, isLoading, isError, error } = useDeviceDetail(id);
  const [renameOpen, setRenameOpen] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);

  if (isLoading) {
    return (
      <PageContainer>
        <Skeleton className="mb-6 h-5 w-32" />
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 space-y-4">
            <Skeleton className="h-48 w-full rounded-lg" />
            <Skeleton className="h-64 w-full rounded-lg" />
          </div>
          <div className="space-y-4">
            <Skeleton className="h-40 w-full rounded-lg" />
          </div>
        </div>
      </PageContainer>
    );
  }

  if (isError || !device) {
    return (
      <PageContainer>
        <Link
          to="/devices"
          className="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to devices
        </Link>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <HardDrive className="h-10 w-10 text-muted-foreground/40" />
            <h3 className="mt-4 text-base font-semibold">
              {error instanceof Error ? error.message : 'Device not found'}
            </h3>
            <Button variant="outline" size="sm" className="mt-4" asChild>
              <Link to="/devices">Back to devices</Link>
            </Button>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  const Icon = typeIcon[device.type] ?? Laptop;

  return (
    <PageContainer>
      <Link
        to="/devices"
        className="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to devices
      </Link>

      <motion.div initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
        <div className="flex flex-col gap-4 pb-6 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-4">
            <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-muted">
              <Icon className="h-7 w-7 text-muted-foreground" />
            </div>
            <div>
              <h1 className="text-2xl font-semibold tracking-tight">{device.deviceName}</h1>
              <p className="text-sm text-muted-foreground">{device.operatingSystem}</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => setRenameOpen(true)}>
              <Pencil className="h-4 w-4" />
              Rename
            </Button>
            <Button variant="destructive" size="sm" onClick={() => setRemoveOpen(true)}>
              <Trash2 className="h-4 w-4" />
              Remove
            </Button>
          </div>
        </div>
      </motion.div>

      <div className="grid gap-6 lg:grid-cols-3">
        {/* Left column: health + info */}
        <div className="space-y-6 lg:col-span-2">
          {/* Health */}
          {/* <Card>
            <CardHeader>
              <CardTitle className="text-base">Device health</CardTitle>
              <CardDescription>Real-time monitoring of this device.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">Health score</span>
                <span className="text-2xl font-semibold" style={{ color: healthColor(device.health) }}>
                  {device.health}%
                </span>
              </div>
              <Progress value={device.health} className="h-2" />
              <div className="grid grid-cols-3 gap-4 pt-2">
                <HealthStat label="Status" value={device.status} variant={statusVariant[device.status]} />
                <HealthStat label="Region" value={device.region} />
                <HealthStat
                  label="Connection"
                  value={device.connectionType === 'lan' ? 'LAN' : 'Remote'}
                />
              </div>
            </CardContent>
          </Card> */}

          {/* Info */}
          {/* <Card>
            <CardHeader>
              <CardTitle className="text-base">Device information</CardTitle>
            </CardHeader>
            <CardContent className="space-y-1">
              <InfoRow icon={Cpu} label="Operating system" value={device.os} />
              <Separator />
              <InfoRow icon={HardDrive} label="Type" value={device.type} className="capitalize" />
              <Separator />
              <InfoRow icon={Globe} label="Region" value={device.region} />
              <Separator />
              <InfoRow
                icon={device.connectionType === 'lan' ? Wifi : WifiOff}
                label="Connection"
                value={device.connectionType === 'lan' ? 'Local network' : 'Remote'}
              />
              <Separator />
              <InfoRow icon={User} label="Owner" value={device.ownerName ?? 'Unassigned'} />
              <Separator />
              <InfoRow
                icon={ArrowUpFromLine}
                label="Current transfer"
                value={device.currentTransfer ?? 'None active'}
              />
            </CardContent>
          </Card> */}

          {/* Recent transfers */}
          {/* <Card>
            <CardHeader>
              <CardTitle className="text-base">Recent transfers</CardTitle>
              <CardDescription>Latest transfer activity on this device.</CardDescription>
            </CardHeader>
            <CardContent className="p-0">
              {device.recentTransfers.length === 0 ? (
                <p className="px-6 py-8 text-center text-sm text-muted-foreground">
                  No recent transfers.
                </p>
              ) : (
                <div className="divide-y">
                  {device.recentTransfers.map((t) => (
                    <div key={t.id} className="flex items-center gap-4 px-6 py-3.5">
                      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted">
                        {t.direction === 'out' ? (
                          <ArrowUpFromLine className="h-4 w-4 text-muted-foreground" />
                        ) : (
                          <ArrowDownToLine className="h-4 w-4 text-muted-foreground" />
                        )}
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium">{t.fileName}</p>
                        <p className="text-xs text-muted-foreground">{formatBytes(t.sizeBytes)}</p>
                      </div>
                      <Badge variant="secondary" className="capitalize">{t.status}</Badge>
                      <span className="hidden text-xs text-muted-foreground sm:inline">
                        {formatRelativeTime(t.createdAt)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card> */}
        </div>

        {/* Right column: owner + status */}
        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Owner</CardTitle>
            </CardHeader>
            <CardContent>
              {device.ownerName ? (
                <div className="flex items-center gap-3">
                  <Avatar className="h-11 w-11">
                    <AvatarFallback className="bg-muted text-sm font-medium">
                      {device.ownerName.split(' ').map((p) => p[0]).join('').slice(0, 2).toUpperCase()}
                    </AvatarFallback>
                  </Avatar>
                  <div>
                    <p className="text-sm font-medium">{device.ownerName}</p>
                    <p className="text-xs text-muted-foreground">Assigned owner</p>
                  </div>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">No owner assigned.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-base">Status</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">Current status</span>
                <Badge variant={statusVariant[device.status] ?? 'default'} className="capitalize">
                  {device.status}
                </Badge>
              </div>
              <Separator />
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">Last seen</span>
                <span className="text-sm font-medium">{formatRelativeTime(device.lastSeenAt)}</span>
              </div>
              <Separator />
              {/* <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">Health</span>
                <span className="text-sm font-semibold" style={{ color: healthColor(device.health) }}>
                  {device.health >= 80 ? 'Healthy' : device.health >= 50 ? 'Warning' : 'Critical'}
                </span>
              </div> */}
            </CardContent>
          </Card>
        </div>
      </div>

      <RenameDeviceDialog open={renameOpen} onOpenChange={setRenameOpen} device={device} />
      <RemoveDeviceDialog
        open={removeOpen}
        onOpenChange={(open) => {
          setRemoveOpen(open);
          if (!open && device) navigate('/devices');
        }}
        device={device}
      />
    </PageContainer>
  );
}

// function HealthStat({
//   label,
//   value,
//   variant,
// }: {
//   label: string;
//   value: string;
//   variant?: 'success' | 'secondary' | 'default';
// }) {
//   return (
//     <div>
//       <p className="text-xs text-muted-foreground">{label}</p>
//       {variant ? (
//         <Badge variant={variant} className="mt-1 capitalize">{value}</Badge>
//       ) : (
//         <p className="mt-1 text-sm font-medium capitalize">{value}</p>
//       )}
//     </div>
//   );
// }

// function InfoRow({
//   icon: Icon,
//   label,
//   value,
//   className,
// }: {
//   icon: LucideIcon;
//   label: string;
//   value: string;
//   className?: string;
// }) {
//   return (
//     <div className="flex items-center justify-between py-3">
//       <div className="flex items-center gap-3">
//         <Icon className="h-4 w-4 text-muted-foreground" />
//         <span className="text-sm text-muted-foreground">{label}</span>
//       </div>
//       <span className={`text-sm font-medium ${className ?? ''}`}>{value}</span>
//     </div>
//   );
// }
