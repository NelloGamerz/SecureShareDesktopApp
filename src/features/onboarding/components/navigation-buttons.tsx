import { ArrowLeft, ArrowRight } from 'lucide-react';
import { Button } from '@/components/ui/button';

interface NavigationButtonsProps {
  onPrevious?: () => void;
  onNext?: () => void;
  nextLabel?: string;
  previousLabel?: string;
  nextDisabled?: boolean;
  nextLoading?: boolean;
  showPrevious?: boolean;
  showNext?: boolean;
}

/** Previous / Next footer row used across all onboarding steps. */
export function NavigationButtons({
  onPrevious,
  onNext,
  nextLabel = 'Next',
  previousLabel = 'Previous',
  nextDisabled,
  nextLoading,
  showPrevious = true,
  showNext = true,
}: NavigationButtonsProps) {
  return (
    <div className="flex items-center justify-between">
      <div>
        {showPrevious && onPrevious && (
          <Button type="button" variant="ghost" onClick={onPrevious} className="gap-1.5">
            <ArrowLeft className="h-4 w-4" />
            {previousLabel}
          </Button>
        )}
      </div>
      <div>
        {showNext && onNext && (
          <Button
            type="button"
            onClick={onNext}
            disabled={nextDisabled || nextLoading}
            className="gap-1.5"
          >
            {nextLabel}
            <ArrowRight className="h-4 w-4" />
          </Button>
        )}
      </div>
    </div>
  );
}
