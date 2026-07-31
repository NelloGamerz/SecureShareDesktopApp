import { Search } from 'lucide-react';
import { useUIStore } from '@/store/ui-store';
import { cn } from '@/lib/utils';

export function SearchTrigger({ className }: { className?: string }) {
  const setCommandOpen = useUIStore((s) => s.setCommandOpen);

  return (
    <button
      onClick={() => setCommandOpen(true)}
      className={cn(
        'group flex h-9 w-full items-center gap-2.5 rounded-md border bg-muted/40 px-3 text-sm text-muted-foreground transition-colors hover:bg-muted/70 md:w-56 lg:w-72',
        className
      )}
    >
      <Search className="h-4 w-4 shrink-0" />
      <span className="flex-1 text-left">Search…</span>
      <kbd className="hidden shrink-0 rounded border bg-background px-1.5 py-0.5 text-[10px] font-medium sm:inline-block">
        ⌘K
      </kbd>
    </button>
  );
}
