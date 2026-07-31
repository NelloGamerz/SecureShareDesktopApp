import { AlertCircle, Inbox, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';

export function DashboardError({
  message,
  onRetry,
}: {
  message?: string;
  onRetry?: () => void;
}) {
  return (
    <div className="flex h-full min-h-[50vh] flex-col items-center justify-center px-6 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-destructive/10 text-destructive">
        <AlertCircle className="h-7 w-7" />
      </div>
      <h3 className="mt-4 text-base font-semibold">Failed to load dashboard</h3>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">
        {message ?? 'Something went wrong while fetching your data. Please try again.'}
      </p>
      {onRetry && (
        <Button variant="outline" size="sm" className="mt-5" onClick={onRetry}>
          <RefreshCw className="h-4 w-4" />
          Try again
        </Button>
      )}
    </div>
  );
}

export function DashboardEmpty({ message }: { message?: string }) {
  return (
    <div className="flex h-full min-h-[40vh] flex-col items-center justify-center px-6 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Inbox className="h-7 w-7" />
      </div>
      <h3 className="mt-4 text-base font-semibold">No data yet</h3>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">
        {message ?? 'There is no data to display. Once activity is recorded it will appear here.'}
      </p>
    </div>
  );
}
