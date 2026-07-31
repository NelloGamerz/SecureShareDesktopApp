import { motion } from 'framer-motion';
import { ArrowLeftRight, CheckCircle2, MailWarning } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { toast } from 'sonner';

export function VerifyEmailPage() {
  const [code, setCode] = useState('');
  const [loading, setLoading] = useState(false);
  const [verified, setVerified] = useState(false);

  const handleVerify = (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setTimeout(() => {
      setLoading(false);
      setVerified(true);
      toast.success('Email verified successfully.');
    }, 800);
  };

  const handleResend = () => {
    toast.info('Verification email resent.');
  };

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background px-6">
      <Link to="/" className="mb-8 flex items-center gap-2.5">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
          <ArrowLeftRight className="h-5 w-5" />
        </div>
        <span className="text-lg font-semibold tracking-tight">Helix</span>
      </Link>

      {verified ? (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="w-full max-w-sm space-y-6 text-center"
        >
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-success/10 text-success">
            <CheckCircle2 className="h-7 w-7" />
          </div>
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">You're verified</h1>
            <p className="mt-1.5 text-sm text-muted-foreground">
              Your email has been confirmed. You can now access your workspace.
            </p>
          </div>
          <Button asChild className="w-full">
            <Link to="/dashboard">Continue to dashboard</Link>
          </Button>
        </motion.div>
      ) : (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="w-full max-w-sm space-y-6"
        >
          <div className="text-center">
            <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
              <MailWarning className="h-7 w-7" />
            </div>
            <h1 className="text-2xl font-semibold tracking-tight">Verify your email</h1>
            <p className="mt-1.5 text-sm text-muted-foreground">
              Enter the 6-digit code we sent to your inbox.
            </p>
          </div>

          <form onSubmit={handleVerify} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="code">Verification code</Label>
              <Input
                id="code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="123456"
                maxLength={6}
                required
                className="text-center text-lg tracking-[0.5em]"
              />
            </div>
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Verifying…' : 'Verify email'}
            </Button>
          </form>

          <div className="flex items-center justify-between text-sm">
            <span className="text-muted-foreground">Didn't get it?</span>
            <button
              onClick={handleResend}
              className="font-medium text-foreground hover:underline"
            >
              Resend code
            </button>
          </div>
        </motion.div>
      )}
    </div>
  );
}
