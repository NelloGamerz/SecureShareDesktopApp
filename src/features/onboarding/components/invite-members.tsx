import { AnimatePresence, motion } from 'framer-motion';
import { Mail, Plus, X } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { emailSchema } from '../onboarding-schemas';

interface InviteMembersProps {
  invites: string[];
  onChange: (invites: string[]) => void;
}

/** Dynamic email-invite list with per-row validation and removal. */
export function InviteMembers({ invites, onChange }: InviteMembersProps) {
  const [input, setInput] = useState('');
  const [error, setError] = useState<string | null>(null);

  const addEmail = () => {
    const trimmed = input.trim();
    if (!trimmed) return;
    const result = emailSchema.safeParse(trimmed);
    if (!result.success) {
      setError(result.error.errors[0]?.message ?? 'Invalid email');
      return;
    }
    if (invites.includes(trimmed.toLowerCase())) {
      setError('This email has already been added');
      return;
    }
    onChange([...invites, trimmed.toLowerCase()]);
    setInput('');
    setError(null);
  };

  const removeEmail = (email: string) => {
    onChange(invites.filter((e) => e !== email));
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      addEmail();
    }
  };

  return (
    <div className="space-y-4">
      {/* Add email row */}
      <div className="flex flex-col gap-2 sm:flex-row">
        <div className="relative flex-1">
          <Mail className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="email"
            placeholder="teammate@company.com"
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
              if (error) setError(null);
            }}
            onKeyDown={handleKeyDown}
            className="pl-9"
            aria-label="Email address"
          />
        </div>
        <Button type="button" variant="outline" onClick={addEmail} className="gap-1.5">
          <Plus className="h-4 w-4" />
          Add another
        </Button>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      {/* Email list */}
      <div className="space-y-2">
        <AnimatePresence initial={false}>
          {invites.map((email) => (
            <motion.div
              key={email}
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, x: -8 }}
              transition={{ duration: 0.15 }}
              className="flex items-center gap-3 rounded-lg border bg-card px-3 py-2.5"
            >
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
                <span className="text-xs font-medium uppercase">
                  {email[0]}
                </span>
              </div>
              <span className="flex-1 truncate text-sm">{email}</span>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-7 w-7 text-muted-foreground hover:text-destructive"
                onClick={() => removeEmail(email)}
                aria-label={`Remove ${email}`}
              >
                <X className="h-3.5 w-3.5" />
              </Button>
            </motion.div>
          ))}
        </AnimatePresence>

        {invites.length === 0 && (
          <p className="rounded-lg border border-dashed py-8 text-center text-sm text-muted-foreground">
            No teammates added yet. You can skip this step and invite later.
          </p>
        )}
      </div>
    </div>
  );
}
