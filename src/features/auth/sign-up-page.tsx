import { useAuth } from '@/contexts/auth-context';
import { BrowserAuthPanel } from './browser-auth-panel';

export function SignUpPage() {
  const { signUp } = useAuth();

  return (
    <BrowserAuthPanel
      title="Create your account"
      subtitle="Set up your workspace and start moving files securely."
      actionLabel="Continue in browser"
      pendingLabel="Waiting for you to finish in your browser…"
      footerText="Already have an account?"
      footerLinkLabel="Sign in"
      footerLinkTo="/sign-in"
      onAuthenticate={() => void signUp()}
    />
  );
}

export default SignUpPage;
