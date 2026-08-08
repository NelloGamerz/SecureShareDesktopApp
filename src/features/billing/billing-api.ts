import { api } from "@/lib/api";
import { PlanType } from "../onboarding/onboarding-types";

export type BillingUsageSummary = {
  seats?: number;
  activeUsers?: number;
  estimatedMonthlyCost?: number | string;
  totalSeats?: number;
  usedSeats?: number;
  monthlyCost?: number | string;
};

export interface BillingInvoice {
  id: string;
  clerkUserId: string;
  subscriptionId: string;
  razorpayPaymentId: string | null;
  razorpaySubscriptionId: string | null;
  razorpayInvoiceId: string | null;
  razorpayOrderId: string | null;
  amount: number;
  currency: string;
  method: string | null;
  status: string;
  description: string | null;
  paidAt: string;
  createdAt: string;
}

export type BillingInvoicesResponse =
  | BillingInvoice[]
  | {
      invoices?: BillingInvoice[];
      data?: BillingInvoice[];
      items?: BillingInvoice[];
    };
export type BillingUsageResponse =
  | BillingUsageSummary
  | {
      usage?: BillingUsageSummary;
      data?: BillingUsageSummary;
      summary?: BillingUsageSummary;
    };

export type RazorpayOrderPayload = {
  billingCycle: "monthly" | "yearly";
  planType?: "individual" | "team";
  planId?: string;
  amount?: number;
  currency?: string;
  description?: string;
};

export type RazorpayOrderResponse = {
  id?: string;
  orderId?: string;
  keyId?: string;
  key?: string;
  amount?: number;
  currency?: string;
  receipt?: string;
  status?: string;
  notes?: Record<string, string>;
};

export interface PageResponse<T> {
  content: T[];
  totalElements: number;
  totalPages: number;
  number: number;
  size: number;
  first: boolean;
  last: boolean;
}

export type RazorpayVerificationPayload = {
  orderId: string;
  paymentId: string;
  signature: string;
  billingCycle?: "monthly" | "yearly";
  planType?: "individual" | "team";
  planId?: string;
};

export type RazorpayVerificationResponse = {
  success: boolean;
  message?: string;
  status?: string;
};

export type UpdateSubscriptionSeatsPayload = {
  seatCount: number;
  planType?: PlanType;
  billingCycle?: "monthly" | "yearly";
};

export type UpdateSubscriptionSeatsResponse = {
  success?: boolean;
  message?: string;
  status?: string;
  seatCount?: number;
  amount?: number;
  currency?: string;
  subscriptionId?: string;
  keyId?: string;
  orderId?: string;
  id?: string;
};

export async function updateSubscriptionSeats(
  payload: UpdateSubscriptionSeatsPayload,
): Promise<UpdateSubscriptionSeatsResponse> {
  const { data } = await api.post<UpdateSubscriptionSeatsResponse>(
    "/subscription/seats",
    payload,
  );
  return data;
}

export async function createRazorpayOrder(
  payload: RazorpayOrderPayload,
): Promise<RazorpayOrderResponse> {
  const { data } = await api.post<RazorpayOrderResponse>(
    "/subscription/create",
    payload,
  );
  return {
    id: data.id ?? data.orderId,
    orderId: data.orderId ?? data.id,
    keyId: data.keyId ?? data.key,
    amount: data.amount,
    currency: data.currency,
    receipt: data.receipt,
    status: data.status,
    notes: data.notes,
  };
}

export type CreateSubscriptionPayload = {
  billingCycle: "monthly" | "yearly";
  planType?: "individual" | "team";
  planId?: string;
  description?: string;
};

export type CreateSubscriptionResponse = {
  subscriptionId?: string;
  keyId?: string;
  status?: string;
  notes?: Record<string, string>;
};

export async function createSubscription(
  payload: CreateSubscriptionPayload,
): Promise<CreateSubscriptionResponse> {
  const { data } = await api.post<CreateSubscriptionResponse>(
    "/subscription/create",
    payload,
  );
  const raw = data as unknown as Record<string, unknown>;
  const subscriptionId =
    typeof raw.subscriptionId === "string"
      ? raw.subscriptionId
      : typeof raw.subscription_id === "string"
        ? raw.subscription_id
        : typeof raw.id === "string"
          ? raw.id
          : undefined;

  const keyId =
    typeof raw.keyId === "string"
      ? raw.keyId
      : typeof raw.key === "string"
        ? raw.key
        : undefined;
  const status = typeof raw.status === "string" ? raw.status : data.status;
  const notes =
    typeof raw.notes === "object" && raw.notes !== null
      ? (raw.notes as Record<string, string>)
      : data.notes;

  return {
    subscriptionId,
    keyId,
    status,
    notes,
  };
}

export type VerifySubscriptionPayload = {
  subscriptionId: string;
  paymentId: string;
  signature: string;
  billingCycle?: "monthly" | "yearly";
  planId?: string;
};

export type VerifySubscriptionResponse = {
  success: boolean;
  message?: string;
  status?: string;
};

export async function verifySubscription(
  payload: VerifySubscriptionPayload,
): Promise<VerifySubscriptionResponse> {
  try {
    const { data } = await api.post<VerifySubscriptionResponse>(
      "/subscription/razorpay/verify-subscription",
      payload,
    );
    return data;
  } catch {
    const { data } = await api.post<VerifySubscriptionResponse>(
      "/subscription/razorpay/verify",
      payload as unknown as Record<string, unknown>,
    );
    return data;
  }
}

export async function verifyRazorpayPayment(
  payload: RazorpayVerificationPayload,
): Promise<RazorpayVerificationResponse> {
  const { data } = await api.post<RazorpayVerificationResponse>(
    "/subscription/razorpay/verify",
    payload,
  );
  return data;
}

export async function fetchBillingUsageSummary(): Promise<BillingUsageSummary> {
  const endpoints = [
    "/subscription/usage-summary",
    "/subscription/usage",
    "/subscription/summary",
  ];

  for (const endpoint of endpoints) {
    try {
      const { data } = await api.get<BillingUsageResponse>(endpoint);
      if (Array.isArray(data)) {
        continue;
      }

      if ("usage" in data && data.usage) {
        return data.usage;
      }

      if ("data" in data && data.data) {
        return data.data;
      }

      if ("summary" in data && data.summary) {
        return data.summary;
      }

      return data as BillingUsageSummary;
    } catch {
      // continue trying the next endpoint if one is unavailable
    }
  }

  return {};
}

export async function fetchBillingInvoices(
  page = 0,
  size = 10,
): Promise<PageResponse<BillingInvoice>> {
  const { data } = await api.get("/subscription/history", {
    params: { page, size },
  });

  return data;
}
