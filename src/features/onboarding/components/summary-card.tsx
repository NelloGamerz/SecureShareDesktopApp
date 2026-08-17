import { Building2, Calendar, Sparkles, User, Users } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import type { OnboardingFormState } from '../onboarding-types';

interface SummaryCardProps {
  form: OnboardingFormState;
}

const industryLabels: Record<string, string> = {
  SOFTWARE: 'Software',
  VIDEO_EDITING: 'Video Editing',
  MARKETING: 'Marketing',
  EDUCATION: 'Education',
  AGENCY: 'Agency',
  MANUFACTURING: 'Manufacturing',
  OTHER: 'Other',
};

function Row({ icon: Icon, label, value }: { icon: LucideIcon; label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-3">
      <div className="flex items-center gap-2.5 text-sm text-muted-foreground">
        <Icon className="h-4 w-4" />
        {label}
      </div>
      <span className="text-sm font-medium">{value}</span>
    </div>
  );
}

/** Final-step summary of the whole onboarding configuration. */
export function SummaryCard({ form }: SummaryCardProps) {
  const isOrg = form.organizationType === 'ORGANIZATION';
  const workspaceName = isOrg ? form.organizationName : form.workspaceName;

  return (
    <div className="overflow-hidden rounded-xl border bg-card">
      <div className="border-b bg-muted/30 px-5 py-3">
        <p className="text-sm font-semibold">Review your workspace</p>
      </div>
      <div className="divide-y px-5">
        <Row icon={isOrg ? Building2 : User} label="Workspace" value={workspaceName || '—'} />
        <Row icon={Sparkles} label="Type" value={isOrg ? 'Organization' : 'Individual'} />
        {isOrg && (
          <>
            <Row icon={Users} label="Team size" value={form.organizationSize || '—'} />
            <Row icon={Building2} label="Industry" value={industryLabels[form.industry] ?? '—'} />
            <Row icon={Calendar} label="Seats" value="3" />
            <Row icon={Calendar} label="Trial" value="14 days" />
            {/* {form.workspaceSlug && (
              <Row icon={Sparkles} label="Slug" value={`vilsend.app/${form.workspaceSlug}`} />
            )} */}
          </>
        )}
      </div>
    </div>
  );
}
