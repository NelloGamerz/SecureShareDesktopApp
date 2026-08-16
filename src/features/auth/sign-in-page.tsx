import { SignIn } from '@clerk/clerk-react';

export function SignInPage() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-50 dark:bg-slate-950 p-4">
      <SignIn
        signUpUrl="/sign-up"
        fallbackRedirectUrl="/onboarding"
        appearance={{
          elements: {
            rootBox: "w-full max-w-md",
            card: "rounded-2xl shadow-xl border border-slate-200 dark:border-slate-800",
            headerTitle: "text-2xl font-bold",
            headerSubtitle: "text-slate-600 dark:text-slate-400",
            formFieldInput:
              "border-slate-200 dark:border-slate-700",
            socialButtonsBlockButton:
              "border-slate-200 dark:border-slate-700",
            formButtonPrimary:
              "bg-indigo-600 hover:bg-indigo-700",
            footerActionLink:
              "text-indigo-600 dark:text-indigo-400",
          },
        }}
      />
    </div>
  );
}

export default SignInPage;