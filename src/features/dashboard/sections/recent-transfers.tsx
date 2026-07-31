import { motion } from 'framer-motion';
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  CircleDot,
} from 'lucide-react';
import { Link } from 'react-router-dom';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import type { RecentTransfer, TransferStatus } from '../dashboard-api';
import { formatBytes, formatRelativeTime } from '@/lib/format';

const statusVariant: Record<TransferStatus, 'default' | 'secondary' | 'success' | 'destructive'> = {
  completed: 'success',
  in_progress: 'secondary',
  queued: 'default',
  failed: 'destructive',
};

const statusLabel: Record<TransferStatus, string> = {
  completed: 'Completed',
  in_progress: 'In progress',
  queued: 'Queued',
  failed: 'Failed',
};

export function RecentTransfers({ transfers }: { transfers: RecentTransfer[] }) {
  return (
    <Card className="lg:col-span-2">
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle className="text-base">Recent transfers</CardTitle>
          <CardDescription>Latest activity across your workspace</CardDescription>
        </div>
        <Button variant="ghost" size="sm" asChild>
          <Link to="/transfers">View all</Link>
        </Button>
      </CardHeader>
      <CardContent className="p-0">
        {transfers.length === 0 ? (
          <p className="px-6 py-10 text-center text-sm text-muted-foreground">
            No transfers yet.
          </p>
        ) : (
          <ScrollArea className="h-[340px]">
            <div className="divide-y">
              {transfers.map((t, i) => (
                <motion.div
                  key={t.id}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ delay: i * 0.03 }}
                  className="group flex items-center gap-4 px-6 py-3.5 transition-colors hover:bg-accent/40"
                >
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted">
                    {t.direction === 'out' ? (
                      <ArrowUpFromLine className="h-4 w-4 text-muted-foreground" />
                    ) : (
                      <ArrowDownToLine className="h-4 w-4 text-muted-foreground" />
                    )}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{t.fileName}</p>
                    <p className="text-xs text-muted-foreground">
                      {formatBytes(t.sizeBytes)} · {t.recipient}
                    </p>
                  </div>
                  <Badge variant={statusVariant[t.status]} className="shrink-0">
                    {statusLabel[t.status]}
                  </Badge>
                  <span className="hidden shrink-0 items-center gap-1 text-xs text-muted-foreground sm:flex">
                    <CircleDot className="h-3 w-3" />
                    {formatRelativeTime(t.createdAt)}
                  </span>
                </motion.div>
              ))}
            </div>
          </ScrollArea>
        )}
      </CardContent>
    </Card>
  );
}
