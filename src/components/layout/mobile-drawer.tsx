import { motion } from "framer-motion";
import { NavLink } from "react-router-dom";
import { Sheet, SheetContent } from "@/components/ui/sheet";
import { navItems } from "@/lib/constants";
import { useUIStore } from "@/store/ui-store";
import { useIsActive, SidebarBrand } from "./sidebar";
import { cn } from "@/lib/utils";
import {
  useCanAccessBilling,
  useCanAccessMembers,
} from "@/features/auth/auth-hooks";

export function MobileDrawer() {
  const open = useUIStore((s) => s.mobileSidebarOpen);
  const setOpen = useUIStore((s) => s.setMobileSidebarOpen);

  const { canAccessBilling } = useCanAccessBilling();
  const { canAccessMembers } = useCanAccessMembers();

  const visibleNavItems = navItems.filter((item) => {
    if (item.to === "/members" && !canAccessMembers) {
      return false;
    }

    return !item.hideForRestrictedMembers || canAccessBilling;
  });

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetContent side="left" className="w-72 p-0">
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="flex h-full flex-col bg-sidebar"
        >
          <SidebarBrand />
          <nav className="flex-1 space-y-1 overflow-y-auto px-3 py-4">
            {visibleNavItems.map((item) => (
              <MobileLink
                key={item.to}
                to={item.to}
                title={item.title}
                icon={item.icon}
                badge={item.badge}
                onNavigate={() => setOpen(false)}
              />
            ))}
          </nav>
        </motion.div>
      </SheetContent>
    </Sheet>
  );
}

function MobileLink({
  to,
  title,
  icon,
  badge,
  onNavigate,
}: {
  to: string;
  title: string;
  icon: (typeof navItems)[number]["icon"];
  badge?: string;
  onNavigate: () => void;
}) {
  const active = useIsActive(to);
  const Icon = icon;

  return (
    <NavLink
      to={to}
      onClick={onNavigate}
      className={cn(
        "flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium transition-colors",
        active
          ? "bg-sidebar-accent text-sidebar-accent-foreground"
          : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
      )}
    >
      <Icon className="h-[1.15rem] w-[1.15rem] shrink-0" />
      <span className="flex-1">{title}</span>
      {badge && (
        <span className="rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-semibold text-primary-foreground">
          {badge}
        </span>
      )}
    </NavLink>
  );
}
