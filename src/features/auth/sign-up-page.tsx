// import {
//   SignUp,
// } from '@clerk/clerk-react';
// import { useState } from 'react';
// import { Link, useNavigate } from 'react-router-dom';
// import { Button } from '@/components/ui/button';
// import { Input } from '@/components/ui/input';
// import { Label } from '@/components/ui/label';
// import { Separator } from '@/components/ui/separator';
// import { toast } from 'sonner';
// import { isClerkConfigured } from '@/lib/env';
// import { AuthSplitLayout } from './auth-split-layout';

// function GoogleIcon() {
//   return (
//     <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
//       <path fill="#EA4335" d="M12 10.2v3.9h5.5c-.24 1.4-1.7 4.1-5.5 4.1-3.3 0-6-2.7-6-6.1s2.7-6.1 6-6.1c1.9 0 3.1.8 3.8 1.5l2.6-2.5C16.9 2.9 14.7 2 12 2 6.9 2 2.8 6.1 2.8 11.1S6.9 20.2 12 20.2c5.8 0 9.6-4 9.6-9.7 0-.65-.07-1.15-.16-1.65H12z" />
//     </svg>
//   );
// }

// function GitHubIcon() {
//   return (
//     <svg viewBox="0 0 24 24" className="h-4 w-4 fill-current" aria-hidden>
//       <path d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.1.68-.22.68-.49 0-.24-.01-.88-.01-1.73-2.78.62-3.37-1.37-3.37-1.37-.45-1.18-1.11-1.49-1.11-1.49-.91-.64.07-.62.07-.62 1 .07 1.53 1.06 1.53 1.06.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.55-1.14-4.55-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.71 0 0 .84-.28 2.75 1.05A9.42 9.42 0 0 1 12 6.84c.85 0 1.71.12 2.51.35 1.91-1.33 2.75-1.05 2.75-1.05.55 1.41.2 2.45.1 2.71.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.81-4.57 5.06.36.32.68.94.68 1.9 0 1.37-.01 2.48-.01 2.82 0 .27.18.6.69.49A10.02 10.02 0 0 0 22 12.25C22 6.58 17.52 2 12 2z" />
//     </svg>
//   );
// }

// export function SignUpPage() {
//   if (isClerkConfigured) {
//     return (
//       <AuthSplitLayout>
//         <div className="clerk-cards">
//           <SignUp routing="path" path="/sign-up" />
//         </div>
//       </AuthSplitLayout>
//     );
//   }
//   return <SignUpPreview />;
// }

// function SignUpPreview() {
//   const navigate = useNavigate();
//   const [loading, setLoading] = useState(false);

//   const handleSubmit = (e: React.FormEvent) => {
//     e.preventDefault();
//     setLoading(true);
//     setTimeout(() => {
//       setLoading(false);
//       toast.success('Account created. Check your email to verify.');
//       navigate('/verify-email');
//     }, 800);
//   };

//   const socialSignUp = (provider: string) => {
//     setLoading(true);
//     setTimeout(() => {
//       setLoading(false);
//       toast.success(`Signed up with ${provider}.`);
//       navigate('/dashboard');
//     }, 800);
//   };

//   return (
//     <AuthSplitLayout>
//       <div className="space-y-6">
//         <div>
//           <h1 className="text-2xl font-semibold tracking-tight">Create your account</h1>
//           <p className="mt-1.5 text-sm text-muted-foreground">
//             Start moving data securely in minutes.
//           </p>
//         </div>

//         <div className="grid grid-cols-2 gap-3">
//           <Button
//             type="button"
//             variant="outline"
//             onClick={() => socialSignUp('Google')}
//             disabled={loading}
//           >
//             <GoogleIcon />
//             Google
//           </Button>
//           <Button
//             type="button"
//             variant="outline"
//             onClick={() => socialSignUp('GitHub')}
//             disabled={loading}
//           >
//             <GitHubIcon />
//             GitHub
//           </Button>
//         </div>

//         <div className="relative">
//           <Separator />
//           <span className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 bg-background px-2 text-xs text-muted-foreground">
//             or sign up with email
//           </span>
//         </div>

//         <form onSubmit={handleSubmit} className="space-y-4">
//           <div className="grid grid-cols-2 gap-3">
//             <div className="space-y-2">
//               <Label htmlFor="firstName">First name</Label>
//               <Input id="firstName" required autoComplete="given-name" />
//             </div>
//             <div className="space-y-2">
//               <Label htmlFor="lastName">Last name</Label>
//               <Input id="lastName" required autoComplete="family-name" />
//             </div>
//           </div>
//           <div className="space-y-2">
//             <Label htmlFor="email">Work email</Label>
//             <Input id="email" type="email" required autoComplete="email" />
//           </div>
//           <div className="space-y-2">
//             <Label htmlFor="password">Password</Label>
//             <Input
//               id="password"
//               type="password"
//               required
//               autoComplete="new-password"
//               placeholder="At least 8 characters"
//             />
//           </div>
//           <Button type="submit" className="w-full" disabled={loading}>
//             {loading ? 'Creating account…' : 'Create account'}
//           </Button>
//         </form>

//         <p className="text-center text-sm text-muted-foreground">
//           Already have an account?{' '}
//           <Link to="/sign-in" className="font-medium text-foreground hover:underline">
//             Sign in
//           </Link>
//         </p>
//       </div>
//     </AuthSplitLayout>
//   );
// }


import { SignUp } from '@clerk/clerk-react';

export function SignUpPage() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-50 dark:bg-slate-950 p-4">
      <SignUp
        signInUrl="/sign-in"
        fallbackRedirectUrl="/onboarding"
        appearance={{
          elements: {
            rootBox: 'w-full max-w-md',
            card: 'rounded-2xl shadow-xl border border-slate-200 dark:border-slate-800',
            headerTitle: 'text-2xl font-bold',
            headerSubtitle: 'text-slate-600 dark:text-slate-400',
            formFieldInput:
              'border-slate-200 dark:border-slate-700',
            socialButtonsBlockButton:
              'border-slate-200 dark:border-slate-700',
            formButtonPrimary: 'bg-indigo-600 hover:bg-indigo-700',
            footerActionLink: 'text-indigo-600 dark:text-indigo-400',
          },
        }}
      />
    </div>
  );
}

export default SignUpPage;