import { motion } from 'framer-motion';
import { ArrowLeftRight } from 'lucide-react';

export function LoadingScreen({ label = 'Loading…' }: { label?: string }) {
  return (
    <div className="flex h-screen w-full flex-col items-center justify-center bg-background">
      <motion.div
        initial={{ opacity: 0, scale: 0.9 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.3 }}
        className="flex flex-col items-center gap-5"
      >
        <div className="relative flex h-14 w-14 items-center justify-center rounded-2xl bg-primary text-primary-foreground">
          <ArrowLeftRight className="h-7 w-7" />
          <motion.span
            className="absolute inset-0 rounded-2xl border-2 border-primary"
            animate={{ scale: [1, 1.35], opacity: [0.6, 0] }}
            transition={{ duration: 1.4, repeat: Infinity, ease: 'easeOut' }}
          />
        </div>
        <div className="flex flex-col items-center gap-2">
          <div className="h-1 w-32 overflow-hidden rounded-full bg-muted">
            <motion.div
              className="h-full w-1/2 rounded-full bg-primary"
              animate={{ x: ['-100%', '200%'] }}
              transition={{ duration: 1.1, repeat: Infinity, ease: 'easeInOut' }}
            />
          </div>
          <p className="text-sm text-muted-foreground">{label}</p>
        </div>
      </motion.div>
    </div>
  );
}

/** Inline spinner for in-page loading states. */
export function PageLoader() {
  return (
    <div className="flex h-full min-h-[40vh] w-full items-center justify-center">
      <motion.div
        animate={{ rotate: 360 }}
        transition={{ duration: 0.9, repeat: Infinity, ease: 'linear' }}
        className="h-7 w-7 rounded-full border-2 border-muted border-t-foreground"
      />
    </div>
  );
}
