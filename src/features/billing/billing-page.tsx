import { motion } from "framer-motion";
import {
  Check,
  CheckCircle2,
  CircleAlert,
  Crown,
  Download,
  Loader2,
  ShieldCheck,
  Sparkles,
  XCircle,
  Zap,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { PageContainer, PageHeader } from "@/components/layout/page-header";
import { useCurrentUserProfile } from "@/features/auth/auth-hooks";
import { useBillingInvoices, useBillingUsageSummary } from "./billing-hooks";
import {
  createSubscription,
  verifySubscription,
  createRazorpayOrder,
  verifyRazorpayPayment,
  updateSubscriptionSeats,
  type BillingInvoice,
} from "./billing-api";
import {
  openSubscriptionCheckout,
  openOrderCheckout,
} from "@/services/razorpay-checkout";

type BillingCycle = "monthly" | "yearly";

function formatCurrency(value?: number | string) {
  if (value === undefined || value === null || value === "") {
    return "—";
  }

  const numericValue = typeof value === "number" ? value : Number(value);
  if (Number.isFinite(numericValue)) {
    return new Intl.NumberFormat("en-IN", {
      style: "currency",
      currency: "INR",
      maximumFractionDigits: 0,
    }).format(numericValue);
  }

  return String(value);
}

export function formatDate(value?: string) {
  if (!value) {
    return "—";
  }

  const parsedDate = new Date(value);
  if (Number.isNaN(parsedDate.getTime())) {
    return value;
  }

  return parsedDate.toLocaleDateString("en-IN", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// function normalizeInvoice(invoice: BillingInvoice, index: number) {
//   const id = invoice.id ?? invoice.invoiceId ?? invoice.invoiceNumber ?? invoice.number ?? `invoice-${index + 1}`;
//   const date = invoice.date ?? invoice.issuedAt ?? invoice.createdAt ?? '';
//   const status = invoice.status ? String(invoice.status) : 'Processed';

//   return {
//     id: String(id),
//     date: formatDate(date),
//     amount: formatCurrency(invoice.amount ?? invoice.total),
//     status: status.charAt(0).toUpperCase() + status.slice(1),
//   };
// }

function normalizeInvoice(invoice: BillingInvoice) {
  return {
    id: invoice.id,
    date: formatDate(invoice.paidAt ?? invoice.createdAt),
    amount: formatCurrency(invoice.amount / 100), // paise -> rupees
    status: invoice.status.charAt(0).toUpperCase() + invoice.status.slice(1),
  };
}

const freeFeatures = [
  "Up to 2 GB per transfer",
  "LAN transfers only",
  "Up to 2 linked devices",
  "Basic transfer history (last 30 days)",
  "Standard transfer speed",
  "Community support",
];

const individualProFeatures = [
  "Unlimited transfer size",
  "LAN + remote transfers",
  "Resume interrupted transfers",
  "Priority transfer performance",
  "Up to 3 linked devices",
  "Unlimited transfer history",
  "Priority support",
  "Future premium features",
];

const teamFeatures = [
  "Everything in Individual Pro",
  "Organization workspace",
  "Team invitations",
  "Role-based permissions",
  "Up to 5 devices per employee",
  "Device management",
  "Centralized billing",
  "Activity logs",
  "Priority support",
];

export function BillingPage() {
  const { data: profile, isLoading } = useCurrentUserProfile();
  const [billingCycle] = useState<BillingCycle>("monthly");
  const [isCheckingOut, setIsCheckingOut] = useState(false);
  const [isSeatDialogOpen, setIsSeatDialogOpen] = useState(false);
  const [selectedSeatCount, setSelectedSeatCount] = useState(1);
  const [isUpdatingSeats, setIsUpdatingSeats] = useState(false);
  const [checkoutStatus, setCheckoutStatus] = useState<
    "idle" | "success" | "error" | "cancelled"
  >("idle");

  const { data: usageSummary = {}, isLoading: isUsageLoading } =
    useBillingUsageSummary();
  // const { data: invoices = [], isLoading: isInvoicesLoading } =
  //   useBillingInvoices();

  const { data: invoicePage, isLoading: isInvoicesLoading } =
    useBillingInvoices();

  const organizationType = profile?.organizationType ?? "INDIVIDUAL";
  const memberRole = profile?.memberRole ?? "OWNER";

  const isIndividualOwner =
    organizationType === "INDIVIDUAL" && memberRole === "OWNER";
  const isOrganizationOwner =
    organizationType === "ORGANIZATION" && memberRole === "OWNER";
  const isRestricted = !isIndividualOwner && !isOrganizationOwner;
  // const normalizedInvoices = invoices.map(normalizeInvoice);
  const normalizedInvoices = invoicePage?.content.map(normalizeInvoice) ?? [];
  const normalizedUsageSummary = usageSummary ?? {};
  const normalizedPlan = String(profile?.plan ?? "FREE").toUpperCase();
  const isFreePlan = normalizedPlan === "FREE" || normalizedPlan === "FREE_TRIAL";
  const isIndividualProPlan = normalizedPlan === "INDIVIDUAL_PRO";
  const currentPeriodEndValue = profile?.currentPeriodEnd
    ? String(profile.currentPeriodEnd)
    : "";
  const parsedCurrentPeriodEnd = currentPeriodEndValue
    ? new Date(currentPeriodEndValue)
    : null;
  const hasValidCurrentPeriodEnd =
    parsedCurrentPeriodEnd instanceof Date &&
    !Number.isNaN(parsedCurrentPeriodEnd.getTime());
  const currentPeriodEndTimestamp = hasValidCurrentPeriodEnd
    ? parsedCurrentPeriodEnd.getTime()
    : null;
  const isCurrentPeriodActive =
    Boolean(currentPeriodEndTimestamp) &&
    (currentPeriodEndTimestamp as number) > Date.now();
  const isProActionDisabled = isIndividualProPlan && isCurrentPeriodActive;
  const renewalMessage = hasValidCurrentPeriodEnd
    ? `Renews on ${formatDate(currentPeriodEndValue)}`
    : "";
  const activeMemberCount = Math.max(
    1,
    Number(
      normalizedUsageSummary.activeUsers ??
        normalizedUsageSummary.usedSeats ??
        normalizedUsageSummary.totalSeats ??
        normalizedUsageSummary.seats ??
        1,
    ) || 1,
  );
  const estimatedSeatCost = selectedSeatCount * 299;

  const handleRazorpayCheckout = async () => {
    // if (!isIndividualOwner) {
    //   toast.error('Razorpay checkout is only available for the individual owner account.');
    //   return;
    // }

    setIsCheckingOut(true);
    setCheckoutStatus("idle");

    try {
      if (isIndividualOwner) {
        const subscription = await createSubscription({
          billingCycle,
          planType: "individual",
          planId: "INDIVIDUAL_PRO",
          description:
            billingCycle === "monthly"
              ? "Individual Pro Monthly Plan"
              : "Individual Pro Yearly Plan",
        });

        if (!subscription.subscriptionId || !subscription.keyId) {
          throw new Error(
            "The backend did not return a valid subscription payload.",
          );
        }

        const resp = await openSubscriptionCheckout({
          key: subscription.keyId,
          subscriptionId: subscription.subscriptionId,
          name: "Startup Server",
          description:
            billingCycle === "monthly"
              ? "Individual Pro Monthly Plan"
              : "Individual Pro Yearly Plan",
          prefill: { name: profile?.firstName ?? undefined },
          notes: { billingCycle, planType: "individual" },
          theme: { color: "#7c3aed" },
        });

        const verification = await verifySubscription({
          subscriptionId: resp.razorpay_subscription_id,
          paymentId: resp.razorpay_payment_id,
          signature: resp.razorpay_signature,
          billingCycle,
          planId: "INDIVIDUAL_PRO",
        });

        if (verification.success) {
          setCheckoutStatus("success");
          toast.success(
            verification.message ?? "Subscription activated successfully.",
          );
        } else {
          setCheckoutStatus("error");
          toast.error(
            verification.message ??
              "Subscription verification failed. Please contact support.",
          );
        }
      } else {
        const amountInPaise = billingCycle === "monthly" ? 19900 : 199000;
        const order = await createRazorpayOrder({
          billingCycle,
          planType: "individual",
          planId: "INDIVIDUAL_PRO",
          amount: amountInPaise,
          currency: "INR",
          description:
            billingCycle === "monthly"
              ? "Individual Pro Monthly Plan"
              : "Individual Pro Yearly Plan",
        });

        if (!order.orderId || !order.keyId) {
          throw new Error(
            "The backend did not return a valid Razorpay order payload.",
          );
        }

        const resp = await openOrderCheckout({
          key: order.keyId,
          orderId: order.orderId,
          amount: order.amount ?? amountInPaise,
          currency: order.currency ?? "INR",
          name: "Startup Server",
          description:
            billingCycle === "monthly"
              ? "Individual Pro Monthly Plan"
              : "Individual Pro Yearly Plan",
          prefill: { name: profile?.firstName ?? undefined },
          notes: { billingCycle, planType: "individual" },
          theme: { color: "#7c3aed" },
        });

        const verification = await verifyRazorpayPayment({
          orderId: resp.razorpay_order_id ?? order.orderId ?? "",
          paymentId: resp.razorpay_payment_id ?? "",
          signature: resp.razorpay_signature ?? "",
          billingCycle,
          planId: "INDIVIDUAL_PRO",
        });

        if (verification.success) {
          setCheckoutStatus("success");
          toast.success(
            verification.message ?? "Payment verified successfully.",
          );
        } else {
          setCheckoutStatus("error");
          toast.error(
            verification.message ??
              "Payment verification failed. Please contact support.",
          );
        }
      }
    } catch (error) {
      setCheckoutStatus("error");
      const message =
        error instanceof Error
          ? error.message
          : "Unable to start the Razorpay checkout flow.";
      toast.error(message);
    } finally {
      setIsCheckingOut(false);
    }
  };

  const handleSeatUpdate = async () => {
    const normalizedSeatCount = Number(selectedSeatCount);

    if (!Number.isFinite(normalizedSeatCount) || normalizedSeatCount < activeMemberCount) {
      toast.error(
        `Seats must be greater than or equal to ${activeMemberCount} active members.`,
      );
      return;
    }

    setIsUpdatingSeats(true);

    try {
      const response = await updateSubscriptionSeats({
        seatCount: normalizedSeatCount,
        planType: "ORGANIZATION",
        billingCycle,
      });

      if (response.success === false) {
        throw new Error(response.message ?? "Unable to update seat plan.");
      }

      const checkoutOrderId = response.orderId ?? response.id;
      const checkoutKeyId = response.keyId;
      const amountInPaise = response.amount;

      if (!checkoutOrderId || !checkoutKeyId) {
        throw new Error(response.message ?? "The seat checkout payload was invalid.");
      }

      await openOrderCheckout({
        key: checkoutKeyId,
        orderId: checkoutOrderId,
        amount: amountInPaise,
        currency: response.currency ?? "INR",
        name: "Startup Server",
        description: `Team seats • ${normalizedSeatCount} seats`,
        prefill: { name: profile?.firstName ?? undefined },
        notes: {
          billingCycle,
          planType: "team",
          seatCount: String(normalizedSeatCount),
        },
        theme: { color: "#7c3aed" },
      });

      toast.success(response.message ?? "Seat checkout opened successfully.");
      setIsSeatDialogOpen(false);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Unable to update seat plan.";
      toast.error(message);
    } finally {
      setIsUpdatingSeats(false);
    }
  };

  if (isLoading) {
    return (
      <PageContainer>
        <PageHeader
          title="Billing"
          description="Loading your billing plan and invoices."
        />
        <div className="space-y-4">
          <Skeleton className="h-36 w-full" />
          <div className="grid gap-4 md:grid-cols-2">
            <Skeleton className="h-56 w-full" />
            <Skeleton className="h-56 w-full" />
          </div>
          <Skeleton className="h-48 w-full" />
        </div>
      </PageContainer>
    );
  }

  if (isRestricted && organizationType === "INDIVIDUAL") {
    return (
      <PageContainer>
        <PageHeader
          title="Billing"
          description="Subscription management for your account."
        />
        <Card className="border-dashed">
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-full bg-primary/10">
              <ShieldCheck className="h-7 w-7 text-primary" />
            </div>
            <h3 className="mt-5 text-lg font-semibold">
              Billing is managed by the account owner.
            </h3>
            <p className="mt-2 max-w-md text-sm text-muted-foreground">
              You don't have permission to manage subscriptions or payment
              methods.
            </p>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  if (isRestricted && organizationType === "ORGANIZATION") {
    return (
      <PageContainer>
        <PageHeader
          title="Billing"
          description="Billing is handled by your organization owner."
        />
        <Card className="border-dashed">
          <CardContent className="flex flex-col items-center justify-center py-16 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-full bg-primary/10">
              <Crown className="h-7 w-7 text-primary" />
            </div>
            <h3 className="mt-5 text-lg font-semibold">
              Billing is managed by your organization owner.
            </h3>
            <p className="mt-2 max-w-md text-sm text-muted-foreground">
              Contact your organization administrator if you need billing
              information.
            </p>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  return (
    <PageContainer>
      <PageHeader
        title="Billing"
        description={
          isIndividualOwner
            ? "Manage your personal plan, invoices, and billing."
            : "Manage your team subscription, seats, and billing."
        }
      />

      {isIndividualOwner ? (
        <>
          <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
          >
            <Card className="mb-6 overflow-hidden border-primary/30 bg-gradient-to-br from-primary/8 via-background to-background">
              <CardContent className="p-0">
                <div className="flex flex-col gap-5 p-6 sm:flex-row sm:items-center sm:justify-between">
                  <div className="flex items-start gap-4">
                    <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
                      <Zap className="h-6 w-6 text-primary" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <h3 className="text-base font-semibold">
                          {isFreePlan ? "Free plan" : "Individual Pro"}
                        </h3>
                        <Badge variant={isFreePlan ? "outline" : "secondary"}>
                          {isFreePlan ? "Current plan" : "Active"}
                        </Badge>
                      </div>
                      <p className="mt-1 text-sm text-muted-foreground">
                        {isFreePlan
                          ? "You are currently on the free plan with essential access."
                          : isIndividualProPlan
                            ? `Individual Pro is active${renewalMessage ? ` • ${renewalMessage}` : ""}`
                            : "Choose a plan that fits your transfer needs."}
                      </p>
                    </div>
                  </div>
                  {/* <div className="flex flex-wrap gap-2">
                    <Button variant="outline" size="sm">
                      Manage billing
                    </Button>
                    <Button
                      size="sm"
                      onClick={handleRazorpayCheckout}
                      disabled={isCheckingOut}
                    >
                      {isCheckingOut ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        "Upgrade / Downgrade"
                      )}
                    </Button>
                  </div> */}
                </div>
              </CardContent>
            </Card>
          </motion.div>

          {/* <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-background/70 p-3">
            <p className="text-sm text-muted-foreground">
              Choose your preferred billing cadence
            </p>
            <div className="flex rounded-full border p-1">
              <Button
                variant={billingCycle === "monthly" ? "default" : "ghost"}
                size="sm"
                onClick={() => setBillingCycle("monthly")}
              >
                Monthly
              </Button>
              <Button
                variant={billingCycle === "yearly" ? "default" : "ghost"}
                size="sm"
                onClick={() => setBillingCycle("yearly")}
              >
                Yearly
              </Button>
            </div>
          </div> */}

          {checkoutStatus !== "idle" && (
            <div className="mb-4 flex items-center gap-2 rounded-lg border p-3 text-sm">
              {checkoutStatus === "success" && (
                <CheckCircle2 className="h-4 w-4 text-emerald-500" />
              )}
              {checkoutStatus === "error" && (
                <CircleAlert className="h-4 w-4 text-destructive" />
              )}
              {checkoutStatus === "cancelled" && (
                <XCircle className="h-4 w-4 text-amber-500" />
              )}
              <span>
                {checkoutStatus === "success" &&
                  "Razorpay payment verified successfully."}
                {checkoutStatus === "error" &&
                  "Razorpay payment could not be verified. Please retry or contact support."}
                {checkoutStatus === "cancelled" &&
                  "The payment window was closed before completion."}
              </span>
            </div>
          )}

          <div className="grid gap-4 lg:grid-cols-2">
            <motion.div
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.05 }}
            >
              <Card
                className={`h-full ${isFreePlan ? "border-primary/40 bg-primary/[0.03]" : "border-border/70"}`}
              >
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Free</CardTitle>
                    <Badge variant={isFreePlan ? "secondary" : "outline"}>
                      {isFreePlan ? "Current plan" : "Starter"}
                    </Badge>
                  </div>
                  <CardDescription className="text-sm">
                    This plan is for students, freelancers, and casual users.
                  </CardDescription>
                  <div className="mt-2 flex items-baseline gap-1">
                    <span className="text-4xl font-semibold tracking-tight">
                      ₹0
                    </span>
                    <span className="text-sm text-muted-foreground">
                      / month
                    </span>
                  </div>
                </CardHeader>
                <CardContent className="space-y-3">
                  {freeFeatures.map((feature) => (
                    <div
                      key={feature}
                      className="flex items-start gap-2.5 text-sm"
                    >
                      <Check className="mt-0.5 h-4 w-4 text-emerald-500" />
                      <span className="text-muted-foreground">{feature}</span>
                    </div>
                  ))}
                  <div className="mt-3 rounded-md border border-dashed p-3 text-sm text-muted-foreground">
                    {isFreePlan
                      ? "You are currently using the free tier."
                      : "Switch to this plan anytime to keep your basic access."}
                  </div>
                </CardContent>
              </Card>
            </motion.div>

            <motion.div
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.1 }}
            >
              <Card
                className={`h-full ${isIndividualProPlan ? "border-primary/40 bg-primary/[0.03]" : "border-border/70"}`}
              >
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Individual Pro</CardTitle>
                    <Badge variant={isIndividualProPlan ? "secondary" : "outline"}>
                      {isIndividualProPlan ? "Current plan" : "Popular"}
                    </Badge>
                  </div>
                  <CardDescription className="text-sm">
                    For power users who need reliable, fast, and flexible
                    transfers.
                  </CardDescription>
                  <div className="mt-2 flex items-baseline gap-1">
                    <span className="text-4xl font-semibold tracking-tight">
                      {billingCycle === "monthly" ? "₹199" : "₹1,990"}
                    </span>
                    <span className="text-sm text-muted-foreground">
                      / {billingCycle === "monthly" ? "month" : "year"}
                    </span>
                  </div>
                  {billingCycle === "yearly" && (
                    <p className="text-sm text-primary">Save 2 months</p>
                  )}
                </CardHeader>
                <CardContent className="space-y-3">
                  {individualProFeatures.map((feature) => (
                    <div
                      key={feature}
                      className="flex items-start gap-2.5 text-sm"
                    >
                      <Check className="mt-0.5 h-4 w-4 text-emerald-500" />
                      <span className="text-muted-foreground">{feature}</span>
                    </div>
                  ))}
                  <Button
                    className="mt-3 w-full"
                    onClick={handleRazorpayCheckout}
                    disabled={isCheckingOut || isProActionDisabled}
                  >
                    {isCheckingOut ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : isIndividualProPlan ? (
                      "Activated"
                    ) : (
                      "Upgrade Plan"
                    )}
                  </Button>
                  {isProActionDisabled && renewalMessage && (
                    <p className="text-center text-xs text-muted-foreground">
                      {renewalMessage}
                    </p>
                  )}
                </CardContent>
              </Card>
            </motion.div>
          </div>

          <div className="mt-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Invoice history</CardTitle>
                <CardDescription>
                  Download your latest invoices anytime.
                </CardDescription>
              </CardHeader>
              <CardContent className="p-0">
                <div className="divide-y">
                  {isInvoicesLoading ? (
                    <div className="px-6 py-8 text-sm text-muted-foreground">
                      Loading invoices…
                    </div>
                  ) : normalizedInvoices.length > 0 ? (
                    normalizedInvoices.map((inv) => (
                      <div
                        key={inv.id}
                        className="flex items-center justify-between px-6 py-3.5 transition-colors hover:bg-accent/40"
                      >
                        <div>
                          <p className="text-sm font-medium">{inv.date}</p>
                          <p className="text-xs text-muted-foreground">
                            {inv.id}
                          </p>
                        </div>
                        <div className="flex items-center gap-4">
                          <span className="text-sm font-medium">
                            {inv.amount}
                          </span>
                          <Badge variant="secondary">{inv.status}</Badge>
                          <Separator orientation="vertical" className="h-5" />
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8"
                          >
                            <Download className="h-4 w-4" />
                          </Button>
                        </div>
                      </div>
                    ))
                  ) : (
                    <div className="px-6 py-8 text-sm text-muted-foreground">
                      No invoices are available yet.
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          </div>
        </>
      ) : (
        <>
          <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
          >
            <Card className="mb-6 overflow-hidden border-primary/30 bg-gradient-to-br from-primary/8 via-background to-background">
              <CardContent className="p-0">
                <div className="flex flex-col gap-5 p-6 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex items-start gap-4">
                    <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
                      <Sparkles className="h-6 w-6 text-primary" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <h3 className="text-base font-semibold">Team plan</h3>
                        <Badge variant="secondary">Active</Badge>
                      </div>
                      <p className="mt-1 text-sm text-muted-foreground">
                        ₹299 per active user / month · 14-day free trial
                      </p>
                    </div>
                  </div>
                  {/* <div className="flex flex-wrap gap-2">
                    <Button variant="outline" size="sm">
                      Manage subscription
                    </Button>
                    <Button variant="outline" size="sm">
                      Manage seats
                    </Button>
                  </div> */}
                </div>
              </CardContent>
            </Card>
          </motion.div>

          <div className="grid gap-4 lg:grid-cols-[1.2fr_0.8fr]">
            <motion.div
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.05 }}
            >
              <Card className="h-full border-primary/40">
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Team</CardTitle>
                    <Badge variant="secondary">Current</Badge>
                  </div>
                  <CardDescription className="text-sm">
                    Everything in Individual Pro plus centralized team controls
                    and workspace features.
                  </CardDescription>
                  <div className="mt-2 flex items-baseline gap-1">
                    <span className="text-4xl font-semibold tracking-tight">
                      ₹299
                    </span>
                    <span className="text-sm text-muted-foreground">
                      per active user / month
                    </span>
                  </div>
                  <p className="text-sm text-primary">14-day free trial</p>
                </CardHeader>
                <CardContent className="space-y-3">
                  {teamFeatures.map((feature) => (
                    <div
                      key={feature}
                      className="flex items-start gap-2.5 text-sm"
                    >
                      <Check className="mt-0.5 h-4 w-4 text-emerald-500" />
                      <span className="text-muted-foreground">{feature}</span>
                    </div>
                  ))}
                  <Dialog
                    open={isSeatDialogOpen}
                    onOpenChange={setIsSeatDialogOpen}
                  >
                    <Button
                      className="mt-3 w-full"
                      onClick={() => {
                        setSelectedSeatCount(activeMemberCount);
                        setIsSeatDialogOpen(true);
                      }}
                    >
                      Pay for seats
                    </Button>
                    <DialogContent className="sm:max-w-md">
                      <DialogHeader>
                        <DialogTitle>Select total seats</DialogTitle>
                        <DialogDescription>
                          Choose the total number of seats for your organization.
                        </DialogDescription>
                      </DialogHeader>
                      <div className="space-y-4">
                        <div className="space-y-2">
                          <label className="text-sm font-medium">
                            Total seats
                          </label>
                          <Input
                            type="number"
                            min={activeMemberCount}
                            value={selectedSeatCount}
                            onChange={(event) => {
                              const parsedValue = Number(event.target.value);
                              setSelectedSeatCount(
                                Number.isFinite(parsedValue) ? parsedValue : 0,
                              );
                            }}
                          />
                          <p className="text-xs text-muted-foreground">
                            Minimum seats must be at least {activeMemberCount} to
                            cover your current active members.
                          </p>
                        </div>
                        <div className="rounded-lg border p-4">
                          <p className="text-sm text-muted-foreground">
                            Estimated monthly total
                          </p>
                          <p className="mt-1 text-2xl font-semibold">
                            {formatCurrency(estimatedSeatCost)}
                          </p>
                          <p className="text-sm text-muted-foreground">
                            ₹299 per seat / month
                          </p>
                        </div>
                      </div>
                      <DialogFooter>
                        <Button
                          variant="outline"
                          onClick={() => setIsSeatDialogOpen(false)}
                        >
                          Cancel
                        </Button>
                        <Button
                          onClick={handleSeatUpdate}
                          disabled={isUpdatingSeats}
                        >
                          {isUpdatingSeats ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            "Pay"
                          )}
                        </Button>
                      </DialogFooter>
                    </DialogContent>
                  </Dialog>
                </CardContent>
              </Card>
            </motion.div>

            <motion.div
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.1 }}
            >
              <Card className="h-full border-border/70">
                <CardHeader>
                  <CardTitle className="text-base">Usage summary</CardTitle>
                  <CardDescription>
                    Current team billing snapshot.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {isUsageLoading ? (
                    <div className="text-sm text-muted-foreground">
                      Loading usage summary…
                    </div>
                  ) : (
                    <>
                      <div className="rounded-lg border p-4">
                        <p className="text-sm text-muted-foreground">Seats</p>
                        <p className="mt-1 text-2xl font-semibold">
                          {normalizedUsageSummary.totalSeats ??
                            normalizedUsageSummary.seats ??
                            "—"}
                        </p>
                      </div>
                      <div className="rounded-lg border p-4">
                        <p className="text-sm text-muted-foreground">
                          Active users
                        </p>
                        <p className="mt-1 text-2xl font-semibold">
                          {normalizedUsageSummary.activeUsers ??
                            normalizedUsageSummary.usedSeats ??
                            "—"}
                        </p>
                      </div>
                      <div className="rounded-lg border p-4">
                        <p className="text-sm text-muted-foreground">
                          Estimated monthly cost
                        </p>
                        <p className="mt-1 text-2xl font-semibold">
                          {formatCurrency(
                            normalizedUsageSummary.estimatedMonthlyCost ??
                              normalizedUsageSummary.monthlyCost,
                          )}
                        </p>
                      </div>
                    </>
                  )}
                </CardContent>
              </Card>
            </motion.div>
          </div>

          <div className="mt-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Invoice history</CardTitle>
                <CardDescription>
                  Download your latest team invoices.
                </CardDescription>
              </CardHeader>
              <CardContent className="p-0">
                <div className="divide-y">
                  {isInvoicesLoading ? (
                    <div className="px-6 py-8 text-sm text-muted-foreground">
                      Loading invoices…
                    </div>
                  ) : normalizedInvoices.length > 0 ? (
                    normalizedInvoices.map((inv) => (
                      <div
                        key={inv.id}
                        className="flex items-center justify-between px-6 py-3.5 transition-colors hover:bg-accent/40"
                      >
                        <div>
                          <p className="text-sm font-medium">{inv.date}</p>
                          <p className="text-xs text-muted-foreground">
                            {inv.id}
                          </p>
                        </div>
                        <div className="flex items-center gap-4">
                          <span className="text-sm font-medium">
                            {inv.amount}
                          </span>
                          <Badge variant="secondary">{inv.status}</Badge>
                          <Separator orientation="vertical" className="h-5" />
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8"
                          >
                            <Download className="h-4 w-4" />
                          </Button>
                        </div>
                      </div>
                    ))
                  ) : (
                    <div className="px-6 py-8 text-sm text-muted-foreground">
                      No invoices are available yet.
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          </div>
        </>
      )}
    </PageContainer>
  );
}
