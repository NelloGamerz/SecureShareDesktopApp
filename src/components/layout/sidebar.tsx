import { motion } from 'framer-motion';
import {
  ArrowLeftRight,
  ChevronsLeft,
  LogOut,
  type LucideIcon,
} from 'lucide-react';
import { NavLink, useLocation } from 'react-router-dom';
import { useCanAccessBilling, useCanAccessMembers } from '@/features/auth/auth-hooks';
import { APP_NAME, navItems } from '@/lib/constants';
import { useUIStore } from '@/store/ui-store';
import { cn } from '@/lib/utils';

export function Sidebar() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const { canAccessBilling } = useCanAccessBilling();
  const { canAccessMembers } = useCanAccessMembers();

  const visibleNavItems = navItems.filter((item) => {
    if (item.to === '/members' && !canAccessMembers) {
      return false;
    }

    return !item.hideForRestrictedMembers || canAccessBilling;
  });

  return (
    <aside
      className={cn(
        'hidden h-screen flex-col border-r bg-sidebar lg:flex',
        collapsed ? 'w-[68px]' : 'w-64'
      )}
    >
      {/* Brand */}
      <div className="flex h-16 shrink-0 items-center gap-2.5 border-b px-4">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground">
          <ArrowLeftRight className="h-5 w-5" />
        </div>
        {!collapsed && (
          <span className="text-base font-semibold tracking-tight">{APP_NAME}</span>
        )}
      </div>

      {/* Nav */}
      <nav className="flex-1 space-y-1 overflow-y-auto px-2.5 py-4">
        {visibleNavItems.map((item) => (
          <SidebarLink key={item.to} item={item} collapsed={collapsed} />
        ))}
      </nav>

      {/* Collapse toggle */}
      <div className="shrink-0 border-t p-2.5">
        <button
          onClick={toggleSidebar}
          className={cn(
            'flex w-full items-center gap-3 rounded-md px-2.5 py-2 text-sm text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
            collapsed && 'justify-center'
          )}
        >
          <ChevronsLeft
            className={cn('h-4 w-4 transition-transform', collapsed && 'rotate-180')}
          />
          {!collapsed && <span>Collapse</span>}
        </button>
      </div>
    </aside>
  );
}

function SidebarLink({
  item,
  collapsed,
}: {
  item: { title: string; to: string; icon: LucideIcon; badge?: string };
  collapsed: boolean;
}) {
  const active = useIsActive(item.to);

  return (
    <NavLink
      to={item.to}
      title={collapsed ? item.title : undefined}
      className={cn(
        'group relative flex items-center gap-3 rounded-md px-2.5 py-2 text-sm font-medium transition-colors',
        collapsed && 'justify-center',
        active
          ? 'bg-sidebar-accent text-sidebar-accent-foreground'
          : 'text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground'
      )}
    >
      {active && (
        <motion.span
          layoutId="sidebar-active"
          className="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-foreground"
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
        />
      )}
      <item.icon className="h-[1.15rem] w-[1.15rem] shrink-0" />
      {!collapsed && <span className="flex-1">{item.title}</span>}
      {!collapsed && item.badge && (
        <span className="rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-semibold text-primary-foreground">
          {item.badge}
        </span>
      )}
    </NavLink>
  );
}

function useIsActive(to: string) {
  const { pathname } = useLocation();
  return pathname === to || pathname.startsWith(`${to}/`);
}

/** Mobile-only brand row used inside the drawer. */
export function SidebarBrand() {
  return (
    <div className="flex h-16 shrink-0 items-center gap-2.5 border-b px-5">
      <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
        <ArrowLeftRight className="h-5 w-5" />
      </div>
      <span className="text-base font-semibold tracking-tight">{APP_NAME}</span>
    </div>
  );
}

/** Sign-out placeholder reused on mobile. */
export function SidebarSignOutButton({
  onClick,
}: {
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
    >
      <LogOut className="h-[1.15rem] w-[1.15rem]" />
      Sign out
    </button>
  );
}

// Re-export for the mobile drawer's active-link detection.
export { useIsActive };
