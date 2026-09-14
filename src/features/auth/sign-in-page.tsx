import { useAuth } from '@/contexts/auth-context';
import { BrowserAuthPanel } from './browser-auth-panel';

export function SignInPage() {
  const { signIn } = useAuth();

  return (
    <BrowserAuthPanel
      title="Welcome back"
      subtitle="Sign in to continue to your workspace and devices."
      actionLabel="Continue in browser"
      pendingLabel="Waiting for you to finish in your browser…"
      footerText="Don't have an account?"
      footerLinkLabel="Create one"
      footerLinkTo="/sign-up"
      onAuthenticate={() => void signIn()}
    />
  );
}

export default SignInPage;
