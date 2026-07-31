import { motion } from 'framer-motion';
import {
  ArrowUpFromLine,
  HardDrive,
  LogIn,
  Settings as SettingsIcon,
  UserPlus,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent } from '@/components/ui/card';
import { PageContainer, PageHeader } from '@/components/layout/page-header';

type ActivityType = 'transfer' | 'device' | 'auth' | 'member' | 'settings';

const events: {
  id: string;
  type: ActivityType;
  actor: string;
  action: string;
  target: string;
  time: string;
}[] = [
  { id: 'a1', type: 'transfer', actor: 'Sarah Kim', action: 'completed transfer of', target: 'Q4-financials.zip', time: '2m ago' },
  { id: 'a2', type: 'device', actor: 'System', action: 'connected new device', target: 'MacBook Pro 16"', time: '14m ago' },
  { id: 'a3', type: 'auth', actor: 'Jordan Avery', action: 'signed in from', target: 'Chrome · macOS', time: '1h ago' },
  { id: 'a4', type: 'member', actor: 'Jordan Avery', action: 'invited', target: 'alex.rivera@helix.industries', time: '2h ago' },
  { id: 'a5', type: 'settings', actor: 'Marc Lefevre', action: 'updated', target: 'organization security policy', time: '4h ago' },
  { id: 'a6', type: 'transfer', actor: 'Priya Nair', action: 'started transfer of', target: 'backup-2026-07.tar.gz', time: '5h ago' },
  { id: 'a7', type: 'member', actor: 'System', action: 'accepted invite for', target: 'alex.rivera@helix.industries', time: '6h ago' },
  { id: 'a8', type: 'device', actor: 'Sarah Kim', action: 'revoked access for', target: 'Pixel 9 Pro', time: '8h ago' },
  { id: 'a9', type: 'settings', actor: 'Jordan Avery', action: 'enabled', target: 'two-factor authentication', time: '1d ago' },
  { id: 'a10', type: 'auth', actor: 'Marc Lefevre', action: 'signed in from', target: 'Safari · macOS', time: '1d ago' },
];

const typeMeta: Record<ActivityType, { icon: typeof LogIn; color: string }> = {
  transfer: { icon: ArrowUpFromLine, color: 'text-chart-1' },
  device: { icon: HardDrive, color: 'text-chart-2' },
  auth: { icon: LogIn, color: 'text-chart-3' },
  member: { icon: UserPlus, color: 'text-chart-4' },
  settings: { icon: SettingsIcon, color: 'text-muted-foreground' },
};

export function ActivityPage() {
  return (
    <PageContainer>
      <PageHeader
        title="Activity"
        description="A complete audit log of everything happening in your workspace."
        actions={<Badge variant="secondary">{events.length} events</Badge>}
      />

      <Card>
        <CardContent className="p-0">
          <div className="relative px-6 py-4">
            {/* timeline rail */}
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
                    className="relative flex items-start gap-4 rounded-lg py-3 pl-0 pr-2 transition-colors hover:bg-accent/40"
                  >
                    <div className="relative z-10 flex h-9 w-9 shrink-0 items-center justify-center rounded-full border-2 border-background bg-muted">
                      <Icon className={`h-4 w-4 ${meta.color}`} />
                    </div>
                    <div className="min-w-0 flex-1 pt-1">
                      <p className="text-sm">
                        <span className="font-medium">{e.actor}</span>{' '}
                        <span className="text-muted-foreground">{e.action}</span>{' '}
                        <span className="font-medium">{e.target}</span>
                      </p>
                      <p className="mt-0.5 text-xs text-muted-foreground">{e.time}</p>
                    </div>
                  </motion.div>
                );
              })}
            </div>
          </div>
        </CardContent>
      </Card>
    </PageContainer>
  );
}
