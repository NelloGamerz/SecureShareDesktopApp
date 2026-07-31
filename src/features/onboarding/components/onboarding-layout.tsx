import { ShieldCheck } from 'lucide-react';
import type { ReactNode } from 'react';

interface OnboardingLayoutProps {
  logo?: ReactNode;
  title: string;
  subtitle: string;
  children: ReactNode;
  footer?: ReactNode;
}

/** Centered, narrow onboarding shell — logo + heading + card slot + footer. */
export function OnboardingLayout({
  logo,
  title,
  subtitle,
  children,
  footer,
}: OnboardingLayoutProps) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background px-4 py-10 sm:px-6">
      <div className="w-full max-w-[700px]">
        {/* Logo */}
        <div className="mb-8 flex items-center justify-center">
          {logo ?? (
            <div className="flex items-center gap-2.5">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
                <ShieldCheck className="h-5 w-5" />
              </div>
              <span className="text-base font-semibold tracking-tight">SecureShare</span>
            </div>
          )}
        </div>

        {/* Heading */}
        <div className="mb-8 text-center">
          <h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">{title}</h1>
          <p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground sm:text-base">
            {subtitle}
          </p>
        </div>

        {/* Card slot */}
        {children}

        {/* Footer (navigation buttons) */}
        {footer && <div className="mt-6">{footer}</div>}
      </div>
    </div>
  );
}
