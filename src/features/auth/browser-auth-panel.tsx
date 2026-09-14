import {
  AlertCircle,
  ExternalLink,
  Loader2,
  ShieldCheck,
  TriangleAlert,
} from 'lucide-react';
import { Link } from 'react-router-dom';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/contexts/auth-context';
import { APP_NAME } from '@/lib/constants';
import { AuthSplitLayout } from './auth-split-layout';

interface BrowserAuthPanelProps {
  title: string;
  subtitle: string;
  actionLabel: string;
  pendingLabel: string;
  footerText: string;
  footerLinkLabel: string;
  footerLinkTo: string;
  onAuthenticate: () => void;
}

/**
 * Sign-in / sign-up screen for the desktop OAuth flow.
 *
 * The user authenticates in the **system browser**, so this screen only starts
 * the flow and reports its state. Clerk's hosted page collects the credentials;
 * nothing is entered inside the app window.
 */
export function BrowserAuthPanel({
  title,
  subtitle,
  actionLabel,
  pendingLabel,
  footerText,
  footerLinkLabel,
  footerLinkTo,
  onAuthenticate,
}: BrowserAuthPanelProps) {
  const {
    isAuthenticating,
    isPreviewMode,
    error,
    cancelAuthentication,
    clearError,
  } = useAuth();

  return (
    <AuthSplitLayout>
      {/*
        Deliberately not wrapped in a framer-motion fade: this panel is the only
        way into the app, and an entrance animation that starts at `opacity: 0`
        leaves the form invisible if the renderer's animation frame is throttled
        or paused. The decorative panel in AuthSplitLayout still animates.
      */}
      <div className="space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
          <p className="mt-1.5 text-sm text-muted-foreground">{subtitle}</p>
        </div>

        {isPreviewMode && (
          <div
            role="status"
            className="flex items-start gap-2.5 rounded-lg border border-warning/30 bg-warning/5 p-3.5 text-sm"
          >
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-warning" />

            <div className="flex-1">
              <p className="font-medium">Authentication is not configured</p>

              <p className="mt-1 text-xs text-muted-foreground">
                Set{' '}
                <code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.7rem]">
                  VITE_CLERK_OAUTH_CLIENT_ID
                </code>{' '}
                in your <code className="font-mono text-[0.7rem]">.env</code> to
                the client ID of your Clerk OAuth application, then restart the
                app.
              </p>
            </div>
          </div>
        )}

        {error && (
          <div
            role="alert"
            className="flex items-start gap-2.5 rounded-lg border border-destructive/30 bg-destructive/5 p-3.5 text-sm text-destructive"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />

            <span className="flex-1">{error}</span>

            <button
              type="button"
              onClick={clearError}
              className="shrink-0 text-xs font-medium underline underline-offset-2"
            >
              Dismiss
            </button>
          </div>
        )}

        {isAuthenticating ? (
          <div className="space-y-4">
            <div className="flex items-center gap-3 rounded-lg border bg-card p-4">
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />

              <span className="text-sm text-muted-foreground">
                {pendingLabel}
              </span>
            </div>

            <p className="text-xs text-muted-foreground">
              Finish signing in on the page that opened in your browser. You can
              close this window and reopen it if you lost the tab.
            </p>

            <Button
              type="button"
              variant="outline"
              className="w-full"
              onClick={() => void cancelAuthentication()}
            >
              Cancel
            </Button>
          </div>
        ) : (
          <div className="space-y-4">
            <Button
              type="button"
              size="lg"
              disabled={isPreviewMode}
              onClick={onAuthenticate}
              className="w-full"
            >
              <ExternalLink className="mr-2 h-4 w-4" />
              {actionLabel}
            </Button>

            <p className="flex items-start gap-2 text-xs text-muted-foreground">
              <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0" />

              <span>
                {APP_NAME} opens your browser to sign you in securely. Your
                password is never entered in this window.
              </span>
            </p>
          </div>
        )}

        <p className="border-t pt-6 text-center text-sm text-muted-foreground">
          {footerText}{' '}
          <Link
            to={footerLinkTo}
            className="font-medium text-foreground underline-offset-4 hover:underline"
          >
            {footerLinkLabel}
          </Link>
        </p>
      </div>
    </AuthSplitLayout>
  );
}
