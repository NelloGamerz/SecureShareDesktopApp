import {
  ArrowLeftRight,
  Bell,
  Check,
  CreditCard,
  HardDrive,
  Trash2,
  Users,
  X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { ScrollArea } from '@/components/ui/scroll-area';
import {
  useNotificationStore,
  type NotificationKind,
} from '@/store/notification-store';
import { cn } from '@/lib/utils';

const kindIcon: Record<NotificationKind, typeof Bell> = {
  transfer: ArrowLeftRight,
  device: HardDrive,
  member: Users,
  billing: CreditCard,
  system: Bell,
};

const kindColor: Record<NotificationKind, string> = {
  transfer: 'text-chart-1',
  device: 'text-chart-2',
  member: 'text-chart-4',
  billing: 'text-chart-3',
  system: 'text-muted-foreground',
};

export function NotificationDropdown() {
  const notifications = useNotificationStore((s) => s.notifications);
  const unreadCount = useNotificationStore((s) => s.unreadCount());
  const markAllRead = useNotificationStore((s) => s.markAllRead);
  const markRead = useNotificationStore((s) => s.markRead);
  const clearAll = useNotificationStore((s) => s.clearAll);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="relative"
          aria-label="Notifications"
        >
          <Bell className="h-[1.15rem] w-[1.15rem]" />
          {unreadCount > 0 && (
            <span className="absolute right-1.5 top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-[10px] font-semibold text-destructive-foreground">
              {unreadCount}
            </span>
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80 p-0">
        <div className="flex items-center justify-between border-b px-3 py-2.5">
          <DropdownMenuLabel className="p-0 text-sm font-semibold">
            Notifications
          </DropdownMenuLabel>
          <div className="flex items-center gap-1">
            <button
              onClick={markAllRead}
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              title="Mark all as read"
            >
              <Check className="h-3.5 w-3.5" />
            </button>
            <button
              onClick={clearAll}
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              title="Clear all"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <ScrollArea className="h-80">
          {notifications.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <Bell className="h-8 w-8 text-muted-foreground/40" />
              <p className="mt-3 text-sm text-muted-foreground">
                You're all caught up.
              </p>
            </div>
          ) : (
            notifications.map((n) => {
              const Icon = kindIcon[n.kind];
              return (
                <div
                  key={n.id}
                  className={cn(
                    'group flex items-start gap-3 border-b px-3 py-3 last:border-b-0 transition-colors hover:bg-accent/50',
                    !n.read && 'bg-accent/30'
                  )}
                >
                  <div className="mt-0.5 shrink-0">
                    <Icon className={cn('h-4 w-4', kindColor[n.kind])} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium">{n.title}</p>
                    <p className="truncate text-xs text-muted-foreground">
                      {n.description}
                    </p>
                    <p className="mt-1 text-[11px] text-muted-foreground/70">
                      {n.timestamp}
                    </p>
                  </div>
                  {!n.read && (
                    <button
                      onClick={() => markRead(n.id)}
                      className="mt-0.5 shrink-0 rounded p-1 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-background"
                      title="Mark as read"
                    >
                      <X className="h-3 w-3 text-muted-foreground" />
                    </button>
                  )}
                </div>
              );
            })
          )}
        </ScrollArea>

        <DropdownMenuSeparator className="my-0" />
        <DropdownMenuItem className="justify-center text-sm text-muted-foreground">
          View all activity
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
