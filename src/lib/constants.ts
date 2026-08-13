import {
  // LayoutDashboard,
  ArrowLeftRight,
  HardDrive,
  Building2,
  Users,
  CreditCard,
  // Activity,
  Settings,
  type LucideIcon,
} from 'lucide-react';

export type NavItem = {
  title: string;
  to: string;
  icon: LucideIcon;
  description: string;
  badge?: string;
  hideForRestrictedMembers?: boolean;
};

export const navItems: NavItem[] = [
  // {
  //   title: 'Dashboard',
  //   to: '/dashboard',
  //   icon: LayoutDashboard,
  //   description: 'Overview of your organization',
  // },
  {
    title: 'Organization',
    to: '/organization',
    icon: Building2,
    description: 'Organization profile and details',
  },
  {
    title: 'Transfers',
    to: '/transfers',
    icon: ArrowLeftRight,
    description: 'File transfer history and status',
  },
  {
    title: 'Devices',
    to: '/devices',
    icon: HardDrive,
    description: 'Manage connected devices',
  },
  {
    title: 'Members',
    to: '/members',
    icon: Users,
    description: 'Team members and roles',
  },
  {
    title: 'Billing',
    to: '/billing',
    icon: CreditCard,
    description: 'Plans, invoices and payment methods',
    hideForRestrictedMembers: true,
  },
  // {
  //   title: 'Activity',
  //   to: '/activity',
  //   icon: Activity,
  //   description: 'Audit log and activity feed',
  // },
  {
    title: 'Settings',
    to: '/settings',
    icon: Settings,
    description: 'Workspace and account settings',
  },
];

export const APP_NAME = 'VilSend';
export const APP_TAGLINE = 'Secure Transfer & Device Management';
