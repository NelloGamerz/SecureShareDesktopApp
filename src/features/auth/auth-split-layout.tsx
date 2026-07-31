import { motion } from 'framer-motion';
import { ArrowLeftRight, ShieldCheck, Zap } from 'lucide-react';
import { Link } from 'react-router-dom';
import { APP_NAME } from '@/lib/constants';

const highlights = [
  {
    icon: ShieldCheck,
    title: 'End-to-end encryption',
    description: 'Every transfer is encrypted in transit and at rest.',
  },
  {
    icon: Zap,
    title: 'Real-time sync',
    description: 'Devices stay in sync across your entire fleet instantly.',
  },
  {
    icon: ArrowLeftRight,
    title: 'Cross-platform',
    description: 'Move files between any device, any OS, anywhere.',
  },
];

export function AuthSplitLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid min-h-screen lg:grid-cols-2">
      {/* Left — form */}
      <div className="flex flex-col justify-center px-6 py-10 sm:px-12 lg:px-16">
        <div className="mx-auto w-full max-w-sm">
          <Link to="/" className="mb-10 flex items-center gap-2.5">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
              <ArrowLeftRight className="h-5 w-5" />
            </div>
            <span className="text-lg font-semibold tracking-tight">{APP_NAME}</span>
          </Link>
          {children}
        </div>
      </div>

      {/* Right — marketing panel */}
      <div className="relative hidden overflow-hidden border-l bg-sidebar lg:flex">
        <div className="absolute inset-0 bg-gradient-to-br from-sidebar via-sidebar to-card" />
        <div
          className="absolute -left-24 top-1/4 h-72 w-72 rounded-full opacity-20 blur-3xl"
          style={{ background: 'hsl(var(--chart-1))' }}
        />
        <div
          className="absolute -right-16 bottom-1/4 h-72 w-72 rounded-full opacity-10 blur-3xl"
          style={{ background: 'hsl(var(--chart-2))' }}
        />
        <div className="relative z-10 flex flex-col justify-center px-16 py-20">
          <h1 className="max-w-md text-3xl font-semibold leading-tight tracking-tight">
            The secure backbone for your organization's transfers and devices.
          </h1>
          <p className="mt-4 max-w-md text-muted-foreground">
            Trusted by teams who move sensitive data between thousands of
            endpoints every day.
          </p>
          <div className="mt-12 space-y-6">
            {highlights.map((h, i) => (
              <motion.div
                key={h.title}
                initial={{ opacity: 0, x: -12 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: 0.15 * i, duration: 0.4 }}
                className="flex items-start gap-4"
              >
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border bg-card">
                  <h.icon className="h-5 w-5" />
                </div>
                <div>
                  <p className="font-medium">{h.title}</p>
                  <p className="text-sm text-muted-foreground">{h.description}</p>
                </div>
              </motion.div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
