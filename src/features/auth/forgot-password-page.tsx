import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { toast } from 'sonner';
import { AuthSplitLayout } from './auth-split-layout';

export function ForgotPasswordPage() {
  const [email, setEmail] = useState('');
  const [sent, setSent] = useState(false);
  const [loading, setLoading] = useState(false);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setTimeout(() => {
      setLoading(false);
      setSent(true);
      toast.success('Reset link sent to your email.');
    }, 700);
  };

  return (
    <AuthSplitLayout>
      <div className="space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Reset your password</h1>
          <p className="mt-1.5 text-sm text-muted-foreground">
            {sent
              ? 'Check your inbox for a link to reset your password.'
              : "Enter your email and we'll send you a reset link."}
          </p>
        </div>

        {sent ? (
          <div className="rounded-lg border bg-card p-5 text-sm">
            <p className="text-muted-foreground">
              We sent a reset link to{' '}
              <span className="font-medium text-foreground">{email}</span>. The link
              expires in 60 minutes.
            </p>
            <Button asChild variant="link" className="mt-3 h-auto p-0">
              <Link to="/sign-in">Back to sign in</Link>
            </Button>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                autoComplete="email"
              />
            </div>
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Sending link…' : 'Send reset link'}
            </Button>
          </form>
        )}

        <p className="text-center text-sm text-muted-foreground">
          Remembered your password?{' '}
          <Link to="/sign-in" className="font-medium text-foreground hover:underline">
            Sign in
          </Link>
        </p>
      </div>
    </AuthSplitLayout>
  );
}
