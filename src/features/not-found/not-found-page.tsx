import { motion } from 'framer-motion';
import { ArrowLeftRight, Compass, Home } from 'lucide-react';
import { Link } from 'react-router-dom';
import { Button } from '@/components/ui/button';

export function NotFoundPage() {
  return (
    <div className="relative flex min-h-screen flex-col items-center justify-center overflow-hidden bg-background px-6">
      {/* ambient glow */}
      <div
        className="absolute -top-32 left-1/2 h-96 w-96 -translate-x-1/2 rounded-full opacity-10 blur-3xl"
        style={{ background: 'hsl(var(--chart-1))' }}
      />

      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4 }}
        className="relative z-10 flex flex-col items-center text-center"
      >
        <Link to="/" className="mb-10 flex items-center gap-2.5">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <ArrowLeftRight className="h-5 w-5" />
          </div>
          <span className="text-lg font-semibold tracking-tight">Helix</span>
        </Link>

        <div className="relative">
          <motion.p
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ delay: 0.1 }}
            className="bg-gradient-to-b from-foreground to-foreground/40 bg-clip-text text-8xl font-bold tracking-tighter text-transparent"
          >
            404
          </motion.p>
          <motion.div
            initial={{ opacity: 0, rotate: -30 }}
            animate={{ opacity: 1, rotate: 0 }}
            transition={{ delay: 0.25, type: 'spring' }}
            className="absolute -right-10 -top-2"
          >
            <Compass className="h-8 w-8 text-muted-foreground" />
          </motion.div>
        </div>

        <h1 className="mt-4 text-xl font-semibold tracking-tight">Page not found</h1>
        <p className="mt-2 max-w-sm text-sm text-muted-foreground">
          The page you're looking for doesn't exist or has been moved. Let's get
          you back on track.
        </p>

        <div className="mt-8 flex items-center gap-3">
          <Button asChild>
            <Link to="/organization">
              <Home className="h-4 w-4" />
              Back to Organization
            </Link>
          </Button>
          <Button variant="outline" asChild>
            <Link to="/sign-in">Sign in</Link>
          </Button>
        </div>
      </motion.div>
    </div>
  );
}
