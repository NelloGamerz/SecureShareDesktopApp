import { motion } from 'framer-motion';
import { Building2, Check, type LucideIcon, User } from 'lucide-react';

interface SelectionCardProps {
  icon: LucideIcon;
  title: string;
  description: string;
  features: string[];
  selected: boolean;
  onSelect: () => void;
}

/** Large selectable card with highlight + check + scale animation. */
export function SelectionCard({
  icon: Icon,
  title,
  description,
  features,
  selected,
  onSelect,
}: SelectionCardProps) {
  return (
    <motion.button
      type="button"
      onClick={onSelect}
      whileHover={{ y: -2 }}
      whileTap={{ scale: 0.98 }}
      animate={{ scale: selected ? 1.01 : 1 }}
      className={`relative flex w-full flex-col rounded-2xl border-2 p-5 text-left transition-all duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background ${
        selected
          ? 'border-primary bg-primary/[0.04]'
          : 'border-border hover:border-foreground/20 hover:bg-accent/30'
      }`}
      aria-pressed={selected}
    >
      <motion.div
        initial={false}
        animate={{ scale: selected ? 1 : 0, opacity: selected ? 1 : 0 }}
        transition={{ duration: 0.15 }}
        className="absolute right-4 top-4 flex h-6 w-6 items-center justify-center rounded-full bg-primary text-primary-foreground"
      >
        <Check className="h-3.5 w-3.5" />
      </motion.div>

      <div
        className={`flex h-11 w-11 items-center justify-center rounded-xl transition-colors ${
          selected ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'
        }`}
      >
        <Icon className="h-5 w-5" />
      </div>

      <h3 className="mt-4 text-base font-semibold">{title}</h3>
      <p className="mt-1 text-sm text-muted-foreground">{description}</p>

      <ul className="mt-4 space-y-2">
        {features.map((f) => (
          <li key={f} className="flex items-center gap-2 text-sm text-muted-foreground">
            <span className="flex h-4 w-4 items-center justify-center rounded-full bg-muted">
              <Check className="h-2.5 w-2.5 text-muted-foreground" />
            </span>
            {f}
          </li>
        ))}
      </ul>
    </motion.button>
  );
}

/** The two usage-type cards shown on Step 1. */
export function UsageSelectionCards({
  value,
  onChange,
}: {
  value: 'INDIVIDUAL' | 'ORGANIZATION' | null;
  onChange: (v: 'INDIVIDUAL' | 'ORGANIZATION') => void;
}) {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <SelectionCard
        icon={User}
        title="Individual"
        description="Perfect for personal file sharing."
        features={['One member', 'Connect desktop & mobile', 'Personal workspace']}
        selected={value === 'INDIVIDUAL'}
        onSelect={() => onChange('INDIVIDUAL')}
      />
      <SelectionCard
        icon={Building2}
        title="Organization"
        description="Collaborate with your team."
        features={['Invite teammates', 'Shared workspace', 'Pay per active member']}
        selected={value === 'ORGANIZATION'}
        onSelect={() => onChange('ORGANIZATION')}
      />
    </div>
  );
}
