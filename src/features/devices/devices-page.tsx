import { motion } from "framer-motion";
import {
  // ArrowLeftRight,
  HardDrive,
  Laptop,
  MoreHorizontal,
  Pencil,
  Plus,
  Server,
  Smartphone,
  Tablet,
  Trash2,
  // Wifi,
  // WifiOff,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import { Link } from "react-router-dom";
// import { Avatar, AvatarFallback } from "@/components/ui/avatar";
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
import { Skeleton } from "@/components/ui/skeleton";
import { PageContainer, PageHeader } from "@/components/layout/page-header";
import { useDevices, useDeviceHealth } from "./devices-hooks";
import { RenameDeviceDialog, RemoveDeviceDialog } from "./devices-dialogs";
import type { Device, DeviceStatus } from "./devices-api";
import { formatRelativeTime } from "@/lib/format";
import { DeviceType } from "../dashboard/dashboard-api";

export const typeIcon: Record<DeviceType, LucideIcon> = {
  laptop: Laptop,
  phone: Smartphone,
  tablet: Tablet,
  server: Server,
  desktop: Laptop,
};

export const typeLabel: Record<DeviceType, string> = {
  laptop: "Laptop",
  phone: "Mobile",
  tablet: "Tablet",
  server: "Server",
  desktop: "Desktop",
};

export const statusVariant: Record<DeviceStatus, "success" | "secondary" | "default"> =
  {
    online: "success",
    idle: "secondary",
    offline: "default",
  };

// function ownerInitials(name: string): string {
//   return name
//     .split(" ")
//     .map((p) => p[0])
//     .join("")
//     .slice(0, 2)
//     .toUpperCase();
// }

export function DevicesPage() {
  const { data: devices, isLoading, isError, error, refetch } = useDevices();
  const { data: health } = useDeviceHealth();

  const [renameDevice, setRenameDevice] = useState<Device | null>(null);
  const [removeDevice, setRemoveDevice] = useState<Device | null>(null);

  const allDevices = devices ?? [];

  return (
    <PageContainer>
      <PageHeader
        title="Devices"
        description="Manage and monitor every device connected to your workspace."
        // actions={
        //   <>
        //     <Button variant="outline" size="sm" asChild>
        //       <Link to="/devices/pair">
        //         <QrCode className="h-4 w-4" />
        //         Pair via QR
        //       </Link>
        //     </Button>
        //     <Button size="sm" asChild>
        //       <Link to="/devices/register">
        //         <Plus className="h-4 w-4" />
        //         Register device
        //       </Link>
        //     </Button>
        //   </>
        // }
      />

      {/* Health summary */}
      {health && (
        <div className="mb-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <HealthChip label="Total" value={health.total} />
          <HealthChip
            label="Online"
            value={health.online}
            accent="text-success"
          />
          <HealthChip
            label="Warning"
            value={health.warning}
            accent="text-warning"
          />
          <HealthChip
            label="Critical"
            value={health.critical}
            accent="text-destructive"
          />
        </div>
      )}

      {isLoading && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 6 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="p-5">
                <div className="flex items-start justify-between">
                  <Skeleton className="h-11 w-11 rounded-xl" />
                  <Skeleton className="h-8 w-8" />
                </div>
                <Skeleton className="mt-4 h-5 w-40" />
                <Skeleton className="mt-1 h-3 w-24" />
                <Skeleton className="mt-4 h-4 w-full" />
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {isError && (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <p className="text-sm text-muted-foreground">
              {error instanceof Error
                ? error.message
                : "Failed to load devices."}
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
      )}

      {!isLoading && !isError && allDevices.length === 0 && (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-full bg-muted">
              <HardDrive className="h-7 w-7 text-muted-foreground" />
            </div>
            <h3 className="mt-4 text-base font-semibold">No devices yet</h3>
            <p className="mt-1 max-w-sm text-sm text-muted-foreground">
              Register a device or pair one via QR code to get started.
            </p>
            <Button size="sm" className="mt-5" asChild>
              <Link to="/devices/register">
                <Plus className="h-4 w-4" />
                Register your first device
              </Link>
            </Button>
          </CardContent>
        </Card>
      )}

      {!isLoading && !isError && allDevices.length > 0 && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {allDevices.map((d, i) => (
            <DeviceCard
              key={d.id}
              device={d}
              index={i}
              onRename={() => setRenameDevice(d)}
              onRemove={() => setRemoveDevice(d)}
            />
          ))}
        </div>
      )}

      <RenameDeviceDialog
        open={renameDevice !== null}
        onOpenChange={(open) => !open && setRenameDevice(null)}
        device={renameDevice}
      />
      <RemoveDeviceDialog
        open={removeDevice !== null}
        onOpenChange={(open) => !open && setRemoveDevice(null)}
        device={removeDevice}
      />
    </PageContainer>
  );
}

function HealthChip({
  label,
  value,
  accent,
}: {
  label: string;
  value: number;
  accent?: string;
}) {
  return (
    <Card>
      <CardContent className="flex items-center justify-between px-4 py-3">
        <span className="text-sm text-muted-foreground">{label}</span>
        <span className={`text-lg font-semibold ${accent ?? ""}`}>{value}</span>
      </CardContent>
    </Card>
  );
}

function DeviceCard({
  device,
  index,
  onRename,
  onRemove,
}: {
  device: Device;
  index: number;
  onRename: () => void;
  onRemove: () => void;
}) {
  const Icon = typeIcon[device.type] ?? Laptop;

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.04 }}
    >
      <Card className="group cursor-pointer transition-all duration-200 hover:-translate-y-1 hover:shadow-lg">
        <CardContent className="space-y-5 p-5">
          {/* Header */}
          <div className="flex items-start justify-between">
            <div className="flex items-center gap-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
                <Icon className="h-6 w-6 text-primary" />
              </div>

              <div className="min-w-0">
                <h3 className="truncate font-semibold">{device.deviceName}</h3>

                <p className="text-xs text-muted-foreground capitalize">
                  {device.operatingSystem.toLowerCase()}
                </p>
              </div>
            </div>

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="opacity-0 transition-opacity group-hover:opacity-100"
                  onClick={(e) => e.preventDefault()}
                >
                  <MoreHorizontal className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>

              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={onRename}>
                  <Pencil className="mr-2 h-4 w-4" />
                  Rename
                </DropdownMenuItem>

                <DropdownMenuSeparator />

                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={onRemove}
                >
                  <Trash2 className="mr-2 h-4 w-4" />
                  Remove
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          {/* Status */}
          <div className="flex items-center justify-between">
            <Badge
              variant={statusVariant[device.status.toLocaleLowerCase() as DeviceStatus]}
              className="flex items-center gap-1.5 capitalize"
            >
              {device.status.toLowerCase()}
            </Badge>

            <Badge variant="outline">
              {typeLabel[device.type.toLowerCase() as DeviceType]}
            </Badge>
          </div>

          {/* Information */}
          <div className="grid grid-cols-2 gap-4 rounded-lg border bg-muted/30 p-3">
            <div>
              <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
                Version
              </p>
              <p className="mt-1 text-sm font-medium">v{device.appVersion}</p>
            </div>

            <div>
              <p className="text-[11px] uppercase tracking-wide text-muted-foreground">
                Registered
              </p>
              <p className="mt-1 text-sm font-medium">
                {formatRelativeTime(device.createdAt)}
              </p>
            </div>
          </div>

          {/* Transfer */}
          {/* {device.currentTransfer ? (
            <div className="flex items-center gap-2 rounded-lg bg-primary/5 px-3 py-2 text-sm">
              <ArrowLeftRight className="h-4 w-4 text-primary" />
              <span className="truncate">{device.currentTransfer}</span>
            </div>
          ) : (
            <div className="rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
              No active transfers
            </div>
          )} */}
        </CardContent>
      </Card>
    </motion.div>
  );
}
