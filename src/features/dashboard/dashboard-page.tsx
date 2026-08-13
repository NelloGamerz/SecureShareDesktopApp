import { motion } from "framer-motion";
import { ArrowUpFromLine, RefreshCw } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { PageContainer, PageHeader } from "@/components/layout/page-header";
import { useDashboard } from "./use-dashboard";
import { DashboardSkeleton } from "./dashboard-skeleton";
import { DashboardError } from "./dashboard-states";
import { StatCards } from "./sections/stat-cards";
import { TransferVolumeChart, DevicesByRegionChart } from "./sections/charts";
import { RecentTransfers } from "./sections/recent-transfers";
import { RecentActivity } from "./sections/recent-activity";
import { QuickActions } from "./sections/quick-actions";
// import { DeviceHealthSection } from "./sections/device-health";
import { useCurrentUserProfile } from "../auth/auth-hooks";
import { useEffect } from "react";

export function DashboardPage() {
  const { data, isLoading, isError, error, refetch, isFetching } =
    useDashboard();
  const { data: profile, isLoading: profileLoading } = useCurrentUserProfile();
  const navigate = useNavigate();

  useEffect(() => {
    if (!profileLoading && profile?.onboardingCompleted === false) {
      navigate("/onboarding", { replace: true });
    }
  }, [profile, profileLoading, navigate]);

  if (isLoading) {
    return <DashboardSkeleton />;
  }

  if (isError || !data) {
    return (
      <DashboardError
        message={error instanceof Error ? error.message : undefined}
        onRetry={() => refetch()}
      />
    );
  }

  const isEmpty =
    data.recentTransfers.length === 0 &&
    data.recentActivity.length === 0 &&
    data.deviceHealth.length === 0;

  return (
    <PageContainer>
      <PageHeader
        title="Dashboard"
        description="Overview of your organization's transfer activity and fleet health."
        actions={
          <>
            {isFetching && (
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <RefreshCw className="h-3 w-3 animate-spin" />
                Updating…
              </span>
            )}
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              <RefreshCw className="h-4 w-4" />
              Refresh
            </Button>
            <Button size="sm" onClick={() => navigate("/transfers")}>
              <ArrowUpFromLine className="h-4 w-4" />
              New transfer
            </Button>
          </>
        }
      />

      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.25 }}
      >
        {/* Statistics cards */}
        <StatCards stats={data.stats} />

        {/* Charts */}
        <div className="mt-4 grid gap-4 lg:grid-cols-3">
          <TransferVolumeChart data={data.transferVolume} />
          <DevicesByRegionChart data={data.devicesByRegion} />
        </div>

        {/* Quick actions */}
        <div className="mt-4">
          <QuickActions />
        </div>

        {/* Recent transfers + Recent activity */}
        <div className="mt-4 grid gap-4 lg:grid-cols-3">
          <RecentTransfers transfers={data.recentTransfers} />
          <RecentActivity events={data.recentActivity} />
        </div>

        {/* Device health */}
        {/* <div className="mt-4">
          <DeviceHealthSection
            devices={data.deviceHealth}
            summary={data.healthSummary}
          />
        </div> */}

        {isEmpty && (
          <p className="mt-6 text-center text-sm text-muted-foreground">
            {import.meta.env.VITE_APP_NAME ?? "VilSend"} is ready — start by
            adding a device or creating a transfer.
          </p>
        )}
      </motion.div>
    </PageContainer>
  );
}
