import { useEffect, type ReactNode } from 'react';
import { useUIStore } from '@/store/ui-store';

/** Applies the persisted theme class to <html> and keeps it in sync. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const theme = useUIStore((s) => s.theme);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.remove('light', 'dark');
    root.classList.add(theme);
  }, [theme]);

  return <>{children}</>;
}
