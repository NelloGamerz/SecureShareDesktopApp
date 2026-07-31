import { Loader2 } from 'lucide-react';
import type { ButtonProps } from '@/components/ui/button';
import { Button } from '@/components/ui/button';

interface LoadingButtonProps extends ButtonProps {
  loading?: boolean;
}

/** Button with a built-in spinner and disabled-while-loading state. */
export function LoadingButton({ loading, children, disabled, ...props }: LoadingButtonProps) {
  return (
    <Button disabled={loading || disabled} {...props}>
      {loading && <Loader2 className="h-4 w-4 animate-spin" />}
      {children}
    </Button>
  );
}
