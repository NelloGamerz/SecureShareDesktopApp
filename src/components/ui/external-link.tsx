import * as React from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';

/**
 * Anchor for links that leave the app.
 *
 * A bare `target="_blank"` is inert inside the Tauri webview: the new-window
 * request has no handler, so the click is silently dropped and nothing opens.
 * The URL has to be handed to the system browser through the opener plugin,
 * which the `opener:default` capability already permits for `https` URLs.
 *
 * Outside Tauri (plain-browser dev runs and the web build) there is no opener
 * command to invoke, so the link falls back to opening a real tab.
 */
export const ExternalLink = React.forwardRef<
  HTMLAnchorElement,
  React.ComponentPropsWithoutRef<'a'>
>(({ href, onClick, ...props }, ref) => {
  const handleClick = (event: React.MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);

    if (!href || event.defaultPrevented) return;

    // Modified clicks mean "do something with the link" (new tab, new window,
    // download). Those are the user's intent to handle, not ours.
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;

    event.preventDefault();

    if (isTauri()) {
      void openUrl(href);
    } else {
      window.open(href, '_blank', 'noopener,noreferrer');
    }
  };

  // `rel` is only meaningful for the browser fallback; Tauri never navigates.
  return (
    <a
      ref={ref}
      href={href}
      target="_blank"
      rel="noreferrer"
      onClick={handleClick}
      {...props}
    />
  );
});

ExternalLink.displayName = 'ExternalLink';
