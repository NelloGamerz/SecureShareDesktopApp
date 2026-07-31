import { Menu } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { Breadcrumbs } from './breadcrumbs';
import { MobileDrawer } from './mobile-drawer';
import { NotificationDropdown } from './notification-dropdown';
import { SearchTrigger } from './search-trigger';
import { ThemeToggle } from './theme-toggle';
import { UserMenu } from './user-menu';
import { CommandPalette } from './command-palette';
import { useUIStore } from '@/store/ui-store';

export function TopNavbar() {
  const setMobileSidebarOpen = useUIStore((s) => s.setMobileSidebarOpen);

  return (
    <>
      <header className="sticky top-0 z-30 flex h-16 shrink-0 items-center gap-3 border-b bg-background/80 px-4 backdrop-blur-md lg:px-6">
        {/* Mobile menu button */}
        <Button
          variant="ghost"
          size="icon"
          className="lg:hidden"
          onClick={() => setMobileSidebarOpen(true)}
          aria-label="Open menu"
        >
          <Menu className="h-5 w-5" />
        </Button>

        {/* Breadcrumbs — hidden on mobile to save space */}
        <Breadcrumbs className="hidden md:flex" />

        <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
          <SearchTrigger className="mx-1" />

          <Separator orientation="vertical" className="h-6" />

          <ThemeToggle />
          <NotificationDropdown />

          <Separator orientation="vertical" className="h-6" />

          <UserMenu />
        </div>
      </header>

      <MobileDrawer />
      <CommandPalette />
    </>
  );
}
