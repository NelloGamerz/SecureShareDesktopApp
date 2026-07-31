import { motion } from 'framer-motion';
import { Check } from 'lucide-react';
import { Button } from '@/components/ui/button';

interface SuccessScreenProps {
  onGoToDashboard: () => void;
}

/** Animated success screen shown after a workspace is created. */
export function SuccessScreen({ onGoToDashboard }: SuccessScreenProps) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      className="flex flex-col items-center justify-center py-10 text-center"
    >
      <motion.div
        initial={{ scale: 0 }}
        animate={{ scale: 1 }}
        transition={{ type: 'spring', stiffness: 260, damping: 18, delay: 0.1 }}
        className="flex h-20 w-20 items-center justify-center rounded-full bg-success/10"
      >
        <motion.div
          initial={{ scale: 0 }}
          animate={{ scale: 1 }}
          transition={{ type: 'spring', stiffness: 300, damping: 16, delay: 0.25 }}
          className="flex h-14 w-14 items-center justify-center rounded-full bg-success text-success-foreground"
        >
          <Check className="h-7 w-7" strokeWidth={3} />
        </motion.div>
      </motion.div>

      <motion.h2
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.4 }}
        className="mt-6 text-2xl font-semibold tracking-tight"
      >
        Workspace Created
      </motion.h2>
      <motion.p
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.5 }}
        className="mt-1.5 text-sm text-muted-foreground"
      >
        Your 14-day free trial has started.
      </motion.p>

      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.6 }}
        className="mt-8"
      >
        <Button onClick={onGoToDashboard} size="lg" className="px-8">
          Go to Dashboard
        </Button>
      </motion.div>
    </motion.div>
  );
}
