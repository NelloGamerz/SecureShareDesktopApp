import { motion } from 'framer-motion';
import { Check } from 'lucide-react';

interface StepIndicatorProps {
  steps: string[];
  current: number;
}

/** Horizontal step indicator with connecting line and completion ticks. */
export function StepIndicator({ steps, current }: StepIndicatorProps) {
  return (
    <div className="mb-8 flex items-center justify-center gap-1.5">
      {steps.map((label, i) => {
        const isComplete = i < current;
        const isActive = i === current;
        return (
          <div key={label} className="flex items-center gap-1.5">
            <div className="flex items-center gap-2">
              <motion.div
                initial={false}
                animate={{
                  scale: isActive ? 1.05 : 1,
                  backgroundColor: isComplete || isActive ? 'hsl(var(--primary))' : 'hsl(var(--muted))',
                  color: isComplete || isActive ? 'hsl(var(--primary-foreground))' : 'hsl(var(--muted-foreground))',
                }}
                transition={{ duration: 0.2 }}
                className="flex h-6 w-6 items-center justify-center rounded-full text-[11px] font-semibold"
              >
                {isComplete ? <Check className="h-3.5 w-3.5" /> : i + 1}
              </motion.div>
              <span
                className={`hidden text-xs font-medium sm:inline ${
                  isActive ? 'text-foreground' : 'text-muted-foreground'
                }`}
              >
                {label}
              </span>
            </div>
            {i < steps.length - 1 && (
              <motion.div
                initial={false}
                animate={{ backgroundColor: isComplete ? 'hsl(var(--primary))' : 'hsl(var(--border))' }}
                className="h-px w-6 sm:w-10"
              />
            )}
          </div>
        );
      })}
    </div>
  );
}
