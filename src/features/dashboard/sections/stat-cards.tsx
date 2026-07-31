import { motion } from 'framer-motion';
import {
  ArrowLeftRight,
  HardDrive,
  HardDriveDownload,
  Users,
  type LucideIcon,
} from 'lucide-react';
import { Card, CardContent } from '@/components/ui/card';
import { Progress } from '@/components/ui/progress';
import type { DashboardStats } from '../dashboard-api';
import { formatNumber } from '@/lib/format';

interface StatCardProps {
  icon: LucideIcon;
  label: string;
  value: string;
  sublabel?: string;
  progress?: number;
  accent: string;
  index: number;
}

function StatCard({ icon: Icon, label, value, sublabel, progress, accent, index }: StatCardProps) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.05 }}
    >
      <Card className="overflow-hidden">
        <CardContent className="p-5">
          <div className="flex items-center justify-between">
            <div
              className="flex h-9 w-9 items-center justify-center rounded-lg"
              style={{ background: `${accent}1a` }}
            >
              <Icon className="h-[1.15rem] w-[1.15rem]" style={{ color: accent }} />
            </div>
          </div>
          <p className="mt-4 text-2xl font-semibold tracking-tight">{value}</p>
          <p className="mt-0.5 text-sm text-muted-foreground">{label}</p>
          {progress !== undefined && (
            <div className="mt-3">
              <Progress value={progress} className="h-1.5" />
              {sublabel && (
                <p className="mt-1.5 text-xs text-muted-foreground">{sublabel}</p>
              )}
            </div>
          )}
          {sublabel && progress === undefined && (
            <p className="mt-1.5 text-xs text-muted-foreground">{sublabel}</p>
          )}
        </CardContent>
      </Card>
    </motion.div>
  );
}

export function StatCards({ stats }: { stats: DashboardStats }) {
  const cards = [
    {
      icon: ArrowLeftRight,
      label: 'Active transfers',
      value: formatNumber(stats.activeTransfers),
      sublabel: 'In progress and queued',
      accent: 'hsl(220, 70%, 55%)',
    },
    {
      icon: HardDrive,
      label: 'Online devices',
      value: `${formatNumber(stats.onlineDevices)}`,
      sublabel: `of ${formatNumber(stats.totalDevices)} connected`,
      accent: 'hsl(160, 60%, 45%)',
    },
    {
      icon: Users,
      label: 'Members',
      value: formatNumber(stats.totalMembers),
      sublabel: 'Active and invited',
      accent: 'hsl(280, 65%, 60%)',
    },
    {
      icon: HardDriveDownload,
      label: 'Storage usage',
      value: `${stats.storageUsedTB} TB`,
      sublabel: `${stats.storagePercent}% of ${stats.storageTotalTB} TB used`,
      progress: stats.storagePercent,
      accent: 'hsl(30, 80%, 55%)',
    },
  ];

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      {cards.map((c, i) => (
        <StatCard key={c.label} {...c} index={i} />
      ))}
    </div>
  );
}
