import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '@/components/ui/command';
import { navItems } from '@/lib/constants';
import { useUIStore } from '@/store/ui-store';
import { useNavigate } from 'react-router-dom';
import { Search } from 'lucide-react';
import { useEffect } from 'react';
import { useCanAccessMembers } from '@/features/auth/auth-hooks';

export function CommandPalette() {
  const open = useUIStore((s) => s.commandOpen);
  const setOpen = useUIStore((s) => s.setCommandOpen);
  const navigate = useNavigate();
  const { canAccessMembers } = useCanAccessMembers();

  // Global ⌘K / Ctrl+K shortcut
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setOpen(!open);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, setOpen]);

  const run = (to: string) => {
    navigate(to);
    setOpen(false);
  };

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Search pages, settings, members…" />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>

        <CommandGroup heading="Navigation">
          {navItems
            .filter((item) => (item.to === '/members' ? canAccessMembers : true))
            .map((item) => (
            <CommandItem
              key={item.to}
              value={`${item.title} ${item.description}`}
              onSelect={() => run(item.to)}
              className="gap-2.5"
            >
              <item.icon className="h-4 w-4 text-muted-foreground" />
              <span>{item.title}</span>
              <span className="ml-auto text-xs text-muted-foreground">
                {item.description}
              </span>
            </CommandItem>
          ))}
        </CommandGroup>

        <CommandSeparator />

        <CommandGroup heading="Quick actions">
          <CommandItem
            value="new transfer"
            onSelect={() => run('/transfers')}
            className="gap-2.5"
          >
            <Search className="h-4 w-4 text-muted-foreground" />
            <span>New transfer</span>
          </CommandItem>
          {canAccessMembers && (
            <CommandItem
              value="invite member"
              onSelect={() => run('/members')}
              className="gap-2.5"
            >
              <Search className="h-4 w-4 text-muted-foreground" />
              <span>Invite a member</span>
            </CommandItem>
          )}
          <CommandItem
            value="billing upgrade plan"
            onSelect={() => run('/billing')}
            className="gap-2.5"
          >
            <Search className="h-4 w-4 text-muted-foreground" />
            <span>Upgrade plan</span>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
