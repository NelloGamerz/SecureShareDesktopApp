import { lazy } from "react";
import { Navigate, RouteObject } from "react-router-dom";

import { AppLayout } from "@/components/layout/app-layout";
import { LoadingScreen } from "@/components/layout/loading-screen";
import {
  ProtectedRoute,
  AuthGate,
  useCurrentUser,
} from "@/providers/auth-guard";

// Auth pages
import { SignInPage } from "@/features/auth/sign-in-page";
import { SignUpPage } from "@/features/auth/sign-up-page";
import { ForgotPasswordPage } from "@/features/auth/forgot-password-page";
import { VerifyEmailPage } from "@/features/auth/verify-email-page";
import { useCanAccessBilling, useCanAccessMembers } from "@/features/auth/auth-hooks";

// Lazy pages
// const DashboardPage = lazy(() =>
//   import("@/features/dashboard/dashboard-page").then((m) => ({
//     default: m.DashboardPage,
//   })),
// );

const TransfersPage = lazy(() =>
  import("@/features/transfers/transfers-page").then((m) => ({
    default: m.TransfersPage,
  })),
);

const DevicesPage = lazy(() =>
  import("@/features/devices/devices-page").then((m) => ({
    default: m.DevicesPage,
  })),
);

// const DeviceDetailPage = lazy(() =>
//   import("@/features/devices/device-detail-page").then((m) => ({
//     default: m.DeviceDetailPage,
//   })),
// );

// const RegisterDevicePage = lazy(() =>
//   import("@/features/devices/register-device-page").then((m) => ({
//     default: m.RegisterDevicePage,
//   })),
// );

const QrPairingPage = lazy(() =>
  import("@/features/devices/qr-pairing-page").then((m) => ({
    default: m.QrPairingPage,
  })),
);

const OrganizationPage = lazy(() =>
  import("@/features/organization/organization-page").then((m) => ({
    default: m.OrganizationPage,
  })),
);

const CreateOrganizationPage = lazy(() =>
  import("@/features/organization/create-organization-page").then((m) => ({
    default: m.CreateOrganizationPage,
  })),
);

const MembersPage = lazy(() =>
  import("@/features/members/members-page").then((m) => ({
    default: m.MembersPage,
  })),
);

const BillingPage = lazy(() =>
  import("@/features/billing/billing-page").then((m) => ({
    default: m.BillingPage,
  })),
);

// const ActivityPage = lazy(() =>
//   import("@/features/activity/activity-page").then((m) => ({
//     default: m.ActivityPage,
//   })),
// );

const SettingsPage = lazy(() =>
  import("@/features/settings/settings-page").then((m) => ({
    default: m.SettingsPage,
  })),
);

const OnboardingPage = lazy(() =>
  import("@/features/onboarding/onboarding-page").then((m) => ({
    default: m.OnboardingPage,
  })),
);

const NotFoundPage = lazy(() =>
  import("@/features/not-found/not-found-page").then((m) => ({
    default: m.NotFoundPage,
  })),
);

function RootRedirect() {
  const { isLoaded, isSignedIn } = useCurrentUser();

  if (!isLoaded) {
    return <LoadingScreen label="Starting VilSend…" />;
  }

  return <Navigate to={isSignedIn ? "/organization" : "/sign-in"} replace />;
}

function BillingRouteGuard() {
  const { isLoading } = useCanAccessBilling();

  if (isLoading) {
    return <LoadingScreen label="Checking access…" />;
  }

  return <BillingPage />;
}

function MembersRouteGuard() {
  const { canAccessMembers, isLoading } = useCanAccessMembers();

  if (isLoading) {
    return <LoadingScreen label="Checking access…" />;
  }

  if (!canAccessMembers) {
    return <Navigate to="/organization" replace />;
  }

  return <MembersPage />;
}

export const router: RouteObject[] = [
  {
    path: "/sign-in",
    element: (
      <AuthGate>
        <SignInPage />
      </AuthGate>
    ),
  },
  {
    path: "/sign-up",
    element: (
      <AuthGate>
        <SignUpPage />
      </AuthGate>
    ),
  },
  {
    path: "/onboarding",
    element: (
      <ProtectedRoute>
        <OnboardingPage />
      </ProtectedRoute>
    ),
  },
  {
    path: "/forgot-password",
    element: <ForgotPasswordPage />,
  },
  {
    path: "/verify-email",
    element: <VerifyEmailPage />,
  },

  {
    element: (
      <ProtectedRoute>
        <AppLayout />
      </ProtectedRoute>
    ),
    children: [
      // {
      //   path: "/dashboard",
      //   element: <DashboardPage />,
      // },
      {
        path: "/transfers",
        element: <TransfersPage />,
      },
      {
        path: "/devices",
        element: <DevicesPage />,
      },
      // {
      //   path: "/devices/register",
      //   element: <RegisterDevicePage />,
      // },
      {
        path: "/devices/pair",
        element: <QrPairingPage />,
      },
      // {
      //   path: "/devices/:id",
      //   element: <DeviceDetailPage />,
      // },
      {
        path: "/organization",
        element: <OrganizationPage />,
      },
      {
        path: "/organization/new",
        element: <CreateOrganizationPage />,
      },
      {
        path: "/members",
        element: <MembersRouteGuard />,
      },
      {
        path: "/billing",
        element: <BillingRouteGuard />,
      },
      // {
      //   path: "/activity",
      //   element: <ActivityPage />,
      // },
      {
        path: "/settings",
        element: <SettingsPage />,
      },
    ],
  },

  {
    path: "/",
    element: <RootRedirect />,
  },

  {
    path: "*",
    element: <NotFoundPage />,
  },
];
