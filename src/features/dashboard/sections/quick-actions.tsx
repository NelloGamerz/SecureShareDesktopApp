import { motion } from 'framer-motion';
import {
  ArrowUpFromLine,
  CreditCard,
  HardDrive,
  UserPlus,
  type LucideIcon,
} from 'lucide-react';
import { Link } from 'react-router-dom';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';

interface QuickAction {
  icon: LucideIcon;
  label: string;
  description: string;
  to: string;
  accent: string;
}

const actions: QuickAction[] = [
  {
    icon: ArrowUpFromLine,
    label: 'New transfer',
    description: 'Send files securely',
    to: '/transfers',
    accent: 'hsl(220, 70%, 55%)',
  },
  {
    icon: HardDrive,
    label: 'Add device',
    description: 'Connect a new endpoint',
    to: '/devices',
    accent: 'hsl(160, 60%, 45%)',
  },
  {
    icon: UserPlus,
    label: 'Invite member',
    description: 'Add a teammate',
    to: '/members',
    accent: 'hsl(280, 65%, 60%)',
  },
  {
    icon: CreditCard,
    label: 'Upgrade plan',
    description: 'Scale your workspace',
    to: '/billing',
    accent: 'hsl(30, 80%, 55%)',
  },
];

export function QuickActions() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Quick actions</CardTitle>
        <CardDescription>Jump to common tasks</CardDescription>
      </CardHeader>
      <CardContent className="grid grid-cols-2 gap-3">
        {actions.map((a, i) => (
          <motion.div
            key={a.label}
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ delay: i * 0.05 }}
          >
            <Link
              to={a.to}
              className="group flex flex-col gap-3 rounded-lg border p-4 transition-all hover:border-foreground/20 hover:bg-accent/40"
            >
              <div
                className="flex h-9 w-9 items-center justify-center rounded-lg transition-transform group-hover:scale-110"
                style={{ background: `${a.accent}1a` }}
              >
                <a.icon className="h-[1.15rem] w-[1.15rem]" style={{ color: a.accent }} />
              </div>
              <div>
                <p className="text-sm font-medium">{a.label}</p>
                <p className="text-xs text-muted-foreground">{a.description}</p>
              </div>
            </Link>
          </motion.div>
        ))}
      </CardContent>
    </Card>
  );
}
