import { Moon, Sun } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useUIStore } from '@/store/ui-store';

export function ThemeToggle() {
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);

  return (
    <Button
      variant="ghost"
      size="icon"
      onClick={toggleTheme}
      aria-label="Toggle theme"
      title="Toggle theme"
    >
      {theme === 'dark' ? (
        <Sun className="h-[1.15rem] w-[1.15rem]" />
      ) : (
        <Moon className="h-[1.15rem] w-[1.15rem]" />
      )}
    </Button>
  );
}
