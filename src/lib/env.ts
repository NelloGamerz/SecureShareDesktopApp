export const env = {
  clerkPublishableKey: import.meta.env.VITE_CLERK_PUBLISHABLE_KEY ?? '',
  apiBaseUrl: import.meta.env.VITE_API_BASE_URL ?? 'http://localhost:8000',
  appName: import.meta.env.VITE_APP_NAME ?? 'VilSend',
} as const;

export const isClerkConfigured = Boolean(
  env.clerkPublishableKey && !env.clerkPublishableKey.startsWith('pk_test_aGVsaXgt')
);
