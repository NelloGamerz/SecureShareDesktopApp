import { AnimatePresence, motion } from "framer-motion";
import { ShieldCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { toast } from "sonner";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import { useCompleteOnboarding } from "./onboarding-hooks";
import type {
  Industry,
  IndividualOnboardingRequest,
  OnboardingFormState,
  OrganizationOnboardingRequest,
  OrganizationSize,
} from "./onboarding-types";
import type { IndividualFormValues } from "./onboarding-schemas";
import type { OrganizationFormValues } from "./onboarding-schemas";
import { OnboardingLayout } from "./components/onboarding-layout";
import { StepIndicator } from "./components/step-indicator";
import { NavigationButtons } from "./components/navigation-buttons";
import { LoadingButton } from "./components/loading-button";
import { UsageSelectionCards } from "./components/selection-card";
import { WorkspaceForm } from "./components/workspace-form";
import { OrganizationForm } from "./components/organization-form";
import { SummaryCard } from "./components/summary-card";
import { SuccessScreen } from "./components/success-screen";
import { getDeviceInfo } from "@/services/getDeviceInfo";
import {
  createDeviceIdentity,
  save_tunnel_hostname,
  save_tunnel_token,
  startCloudflared,
  startTauriWebSocket,
} from "@/api/tauri";
import { useCurrentUserProfile } from "../auth/auth-hooks";

const initialForm: OnboardingFormState = {
  organizationType: null,
  workspaceName: "",
  organizationName: "",
  organizationSize: "",
  industry: "",
  workspaceSlug: "",
  invites: [],
  agreedToTerms: false,
};

export function OnboardingPage() {
  const navigate = useNavigate();
  const { data: profile, isLoading } = useCurrentUserProfile();
  const mutation = useCompleteOnboarding();

  const [step, setStep] = useState(0);
  const [form, setForm] = useState<OnboardingFormState>(initialForm);
  const [individualValid, setIndividualValid] = useState(false);
  const [orgValid, setOrgValid] = useState(false);

  const isOrg = form.organizationType === "ORGANIZATION";
  const steps = useMemo(
    () => ["Usage", "Details", "Review"],
    [],
  );
  const finalStepIndex = steps.length - 1;
  const [success, setSuccess] = useState(false);

  // Prefill individual workspace name from the user's first name.
  const firstName = profile?.firstName ?? "";
  const prefillWorkspace = firstName
    ? `${firstName}'s Workspace`
    : "Your Workspace";

  // If the profile says onboarding is already complete, skip straight to dashboard.
  useEffect(() => {
    if (profile?.onboardingCompleted) {
      navigate("/organization", { replace: true });
    }
  }, [profile, navigate]);

  const updateForm = (patch: Partial<OnboardingFormState>) =>
    setForm((prev) => ({ ...prev, ...patch }));

  // --- Step gating ---
  const canAdvance = (() => {
    if (step === 0) return form.organizationType !== null;
    if (step === 1) return isOrg ? orgValid : individualValid;
    return true;
  })();

  const handleNext = () => {
    if (step < finalStepIndex) setStep((s) => s + 1);
  };
  const handlePrevious = () => {
    if (step > 0) setStep((s) => s - 1);
  };

  // --- Final submission ---
  const handleSubmit = async () => {
    if (!form.agreedToTerms) {
      toast.error("Please agree to the Terms and Privacy Policy to continue.");
      return;
    }

    const publicKey = await createDeviceIdentity();
    const device = await getDeviceInfo();


    let payload: IndividualOnboardingRequest | OrganizationOnboardingRequest;
    if (isOrg) {
      payload = {
        organizationName: form.organizationName.trim(),
        organizationType: "ORGANIZATION",
        organizationSize: form.organizationSize as OrganizationSize,
        industry: form.industry as Industry,
        workspaceSlug: form.workspaceSlug || undefined,
        invites: form.invites,
        device,
        publicKey: publicKey,
      };
    } else {
      payload = {
        organizationName: form.workspaceName.trim(),
        organizationType: "INDIVIDUAL",
        device,
        publicKey: publicKey,
      };
    }

    mutation.mutate(payload, {
      onSuccess: async (response) => {
        if (response.tunnelToken) {
          try {
            await save_tunnel_token(response.tunnelToken);
            await save_tunnel_hostname(response.hostname);
          } catch (error) {
            console.error("Failed to save tunnel token:", error);
            toast.error("Failed to configure device tunnel.");
            return;
          }
        }

        await startCloudflared();
        await startTauriWebSocket(device);

        setSuccess(true);

        toast.success("Workspace created. Your 14-day free trial has started.");

        setTimeout(() => {
          navigate("/organization", { replace: true });
        }, 2000);
      },

      onError: (err) => {
        const status = (err as Error & { status?: number }).status;

        const map: Record<number, string> = {
          400: "Validation failed. Please check your inputs.",
          401: "Your session has expired. Please sign in again.",
          403: "You do not have permission to do this.",
          404: "Endpoint not found.",
          409: "A workspace with this name or slug already exists.",
          500: "Something went wrong on our end. Please try again.",
        };

        toast.error(map[status ?? 500] ?? err.message);
      },
    });
  };

  // --- Loading state ---
  if (isLoading) {
    return (
      <OnboardingLayout title="" subtitle="">
        <Card>
          <CardContent className="space-y-4 p-6">
            <Skeleton className="h-8 w-48 mx-auto" />
            <Skeleton className="h-4 w-64 mx-auto" />
            <div className="grid gap-4 sm:grid-cols-2 pt-4">
              <Skeleton className="h-40 w-full rounded-2xl" />
              <Skeleton className="h-40 w-full rounded-2xl" />
            </div>
          </CardContent>
        </Card>
      </OnboardingLayout>
    );
  }

  // --- Success screen ---
  if (success) {
    return (
      <OnboardingLayout title="" subtitle="">
        <Card>
          <CardContent className="p-6">
            <SuccessScreen
              onGoToDashboard={() =>
                navigate("/organization", { replace: true })
              }
            />
          </CardContent>
        </Card>
      </OnboardingLayout>
    );
  }

  const stepTitle =
    [
      "How will you use VilSend?",
      isOrg ? "Tell us about your organization" : "Name your workspace",
      "Review and create",
    ][step] ?? "";

  const stepSubtitle =
    [
      "Choose the option that fits how you work.",
      isOrg
        ? "This information shapes your workspace."
        : "Pick a name for your personal space.",
      "Confirm your setup before we create the workspace.",
    ][step] ?? "";

  return (
    <OnboardingLayout
      logo={
        <div className="flex items-center gap-2.5">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <ShieldCheck className="h-5 w-5" />
          </div>
          <span className="text-base font-semibold tracking-tight">
            VilSend
          </span>
        </div>
      }
      title="Welcome to VilSend"
      subtitle="Let's set up your workspace."
      footer={
        success ? null : (
          <NavigationButtons
            onPrevious={handlePrevious}
            onNext={handleNext}
            showPrevious={step > 0}
            showNext={step < finalStepIndex}
            nextDisabled={!canAdvance}
            nextLabel={step === finalStepIndex - 1 ? "Review" : "Next"}
          />
        )
      }
    >
      <StepIndicator steps={steps} current={step} />

      <Card>
        <CardContent className="p-6 sm:p-8">
          <AnimatePresence mode="wait">
            <motion.div
              key={step}
              initial={{ opacity: 0, x: 12 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -12 }}
              transition={{ duration: 0.2 }}
            >
              {/* Step heading */}
              <div className="mb-6">
                <h2 className="text-lg font-semibold tracking-tight sm:text-xl">
                  {stepTitle}
                </h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  {stepSubtitle}
                </p>
              </div>

              {/* Step 1: usage type */}
              {step === 0 && (
                <UsageSelectionCards
                  value={form.organizationType}
                  onChange={(v) => updateForm({ organizationType: v })}
                />
              )}

              {/* Step 2: individual OR organization details */}
              {step === 1 && !isOrg && (
                <WorkspaceForm
                  prefill={prefillWorkspace}
                  onChange={(values: IndividualFormValues | null) => {
                    setIndividualValid(Boolean(values));
                    if (values)
                      updateForm({ workspaceName: values.workspaceName });
                  }}
                />
              )}
              {step === 1 && isOrg && (
                <OrganizationForm
                  onChange={(values: OrganizationFormValues | null) => {
                    setOrgValid(Boolean(values));
                    if (values) {
                      updateForm({
                        organizationName: values.organizationName,
                        organizationSize: (values.organizationSize ||
                          "") as OnboardingFormState["organizationSize"],
                        industry: (values.industry ||
                          "") as OnboardingFormState["industry"],
                        workspaceSlug: values.workspaceSlug ?? "",
                      });
                    }
                  }}
                />
              )}

              {/* Final step: summary + terms + submit */}
              {step === finalStepIndex && (
                <div className="space-y-5">
                  <SummaryCard form={form} />

                  <label
                    htmlFor="terms"
                    className="flex cursor-pointer items-start gap-3 rounded-lg border bg-card p-4 transition-colors hover:bg-accent/30"
                  >
                    <Checkbox
                      id="terms"
                      checked={form.agreedToTerms}
                      onCheckedChange={(v) =>
                        updateForm({ agreedToTerms: v === true })
                      }
                      className="mt-0.5"
                    />
                    <span className="text-sm text-muted-foreground">
                      I agree to the{" "}
                      <a
                        href="https://www.vilsend.in/terms"
                        target="_blank"
                        rel="noreferrer"
                        className="font-medium text-foreground underline-offset-4 hover:underline"
                        onClick={(e) => e.stopPropagation()}
                      >
                        Terms of Service
                      </a>{" "}
                      and{" "}
                      <a
                        href="https://www.vilsend.in/privacy"
                        target="_blank"
                        rel="noreferrer"
                        className="font-medium text-foreground underline-offset-4 hover:underline"
                        onClick={(e) => e.stopPropagation()}
                      >
                        Privacy Policy
                      </a>
                      .
                    </span>
                  </label>

                  <LoadingButton
                    type="button"
                    className="w-full"
                    size="lg"
                    loading={mutation.isPending}
                    disabled={!form.agreedToTerms}
                    onClick={handleSubmit}
                  >
                    Create Workspace
                  </LoadingButton>
                </div>
              )}
            </motion.div>
          </AnimatePresence>
        </CardContent>
      </Card>
    </OnboardingLayout>
  );
}
