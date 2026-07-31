import { motion } from 'framer-motion';
import {
  ArrowUpFromLine,
  HardDrive,
  UserPlus,
  Users,
  type LucideIcon,
} from 'lucide-react';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import type { ActivityEvent, ActivityType } from '../dashboard-api';
import { formatRelativeTime } from '@/lib/format';

const typeMeta: Record<
  ActivityType,
  { icon: LucideIcon; color: string }
> = {
  transfer: { icon: ArrowUpFromLine, color: 'text-chart-1' },
  device: { icon: HardDrive, color: 'text-chart-2' },
  member: { icon: UserPlus, color: 'text-chart-4' },
  system: { icon: Users, color: 'text-muted-foreground' },
};

export function RecentActivity({ events }: { events: ActivityEvent[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Recent activity</CardTitle>
        <CardDescription>Latest events in your workspace</CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        {events.length === 0 ? (
          <p className="px-6 py-10 text-center text-sm text-muted-foreground">
            No recent activity.
          </p>
        ) : (
          <ScrollArea className="h-[340px]">
            <div className="relative px-6 py-4">
              <div className="absolute left-[34px] top-2 bottom-2 w-px bg-border" />
              <div className="space-y-1">
                {events.map((e, i) => {
                  const meta = typeMeta[e.type];
                  const Icon = meta.icon;
                  return (
                    <motion.div
                      key={e.id}
                      initial={{ opacity: 0, x: -6 }}
                      animate={{ opacity: 1, x: 0 }}
                      transition={{ delay: i * 0.04 }}
                      className="relative flex items-start gap-4 rounded-lg py-2.5 transition-colors hover:bg-accent/40"
                    >
                      <div className="relative z-10 flex h-8 w-8 shrink-0 items-center justify-center rounded-full border-2 border-background bg-muted">
                        <Icon className={`h-3.5 w-3.5 ${meta.color}`} />
                      </div>
                      <div className="min-w-0 flex-1 pt-0.5">
                        <p className="text-sm font-medium">{e.title}</p>
                        <p className="truncate text-xs text-muted-foreground">
                          {e.description}
                        </p>
                        <p className="mt-0.5 text-[11px] text-muted-foreground/70">
                          {formatRelativeTime(e.timestamp)}
                        </p>
                      </div>
                    </motion.div>
                  );
                })}
              </div>
            </div>
          </ScrollArea>
        )}
      </CardContent>
    </Card>
  );
}
