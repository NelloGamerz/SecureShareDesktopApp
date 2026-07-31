import { motion } from 'framer-motion';
import { CheckCircle2, AlertTriangle, XCircle, Laptop} from 'lucide-react';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Progress } from '@/components/ui/progress';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Badge } from '@/components/ui/badge';
import type { DeviceHealth as DeviceHealthItem, HealthSummary } from '../dashboard-api';
import { formatRelativeTime } from '@/lib/format';
import { statusVariant, typeIcon } from '@/features/devices/devices-page';


function healthColor(health: number): string {
  if (health >= 80) return 'hsl(var(--success))';
  if (health >= 50) return 'hsl(var(--warning))';
  return 'hsl(var(--destructive))';
}

export function DeviceHealthSection({
  devices,
  summary,
}: {
  devices: DeviceHealthItem[];
  summary: HealthSummary;
}) {
  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="text-base">Device health</CardTitle>
            <CardDescription>Monitoring {devices.length} devices</CardDescription>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <CheckCircle2 className="h-3.5 w-3.5 text-success" />
              {summary.healthy} healthy
            </span>
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <AlertTriangle className="h-3.5 w-3.5 text-warning" />
              {summary.warning} warning
            </span>
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <XCircle className="h-3.5 w-3.5 text-destructive" />
              {summary.critical} critical
            </span>
          </div>
        </div>
      </CardHeader>
      <CardContent className="p-0">
        {devices.length === 0 ? (
          <p className="px-6 py-10 text-center text-sm text-muted-foreground">
            No devices registered.
          </p>
        ) : (
          <ScrollArea className="h-[340px]">
            <div className="divide-y">
              {devices.map((d, i) => {
                const Icon = typeIcon[d.type] ?? Laptop;
                return (
                  <motion.div
                    key={d.id}
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    transition={{ delay: i * 0.03 }}
                    className="flex items-center gap-4 px-6 py-3.5 transition-colors hover:bg-accent/40"
                  >
                    <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted">
                      <Icon className="h-4 w-4 text-muted-foreground" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">{d.name}</p>
                      <p className="text-xs text-muted-foreground">
                        {d.os} · {d.region} · {formatRelativeTime(d.lastSeenAt)}
                      </p>
                    </div>
                    <div className="hidden w-24 shrink-0 sm:block">
                      <Progress value={d.health} className="h-1.5" />
                    </div>
                    <span
                      className="shrink-0 text-sm font-semibold tabular-nums"
                      style={{ color: healthColor(d.health) }}
                    >
                      {d.health}%
                    </span>
                    <Badge variant={statusVariant[d.status] ?? 'default'} className="shrink-0">
                      {d.status}
                    </Badge>
                  </motion.div>
                );
              })}
            </div>
          </ScrollArea>
        )}
      </CardContent>
    </Card>
  );
}
