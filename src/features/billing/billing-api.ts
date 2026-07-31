import { api } from '@/lib/api';

export type BillingUsageSummary = {
  seats?: number;
  activeUsers?: number;
  estimatedMonthlyCost?: number | string;
  totalSeats?: number;
  usedSeats?: number;
  monthlyCost?: number | string;
};

export type BillingInvoice = {
  id?: string;
  invoiceId?: string;
  invoiceNumber?: string;
  number?: string;
  date?: string;
  issuedAt?: string;
  createdAt?: string;
  amount?: number | string;
  total?: number | string;
  status?: string;
};

export type BillingInvoicesResponse = BillingInvoice[] | { invoices?: BillingInvoice[]; data?: BillingInvoice[]; items?: BillingInvoice[] };
export type BillingUsageResponse = BillingUsageSummary | { usage?: BillingUsageSummary; data?: BillingUsageSummary; summary?: BillingUsageSummary };

export async function fetchBillingUsageSummary(): Promise<BillingUsageSummary> {
  const endpoints = ['/billing/usage-summary', '/billing/usage', '/billing/summary'];

  for (const endpoint of endpoints) {
    try {
      const { data } = await api.get<BillingUsageResponse>(endpoint);
      if (Array.isArray(data)) {
        continue;
      }

      if ('usage' in data && data.usage) {
        return data.usage;
      }

      if ('data' in data && data.data) {
        return data.data;
      }

      if ('summary' in data && data.summary) {
        return data.summary;
      }

      return data as BillingUsageSummary;
    } catch {
      // continue trying the next endpoint if one is unavailable
    }
  }

  return {};
}

export async function fetchBillingInvoices(): Promise<BillingInvoice[]> {
  const endpoints = ['/billing/invoices', '/billing/invoice-history', '/billing/invoices/history'];

  for (const endpoint of endpoints) {
    try {
      const { data } = await api.get<BillingInvoicesResponse>(endpoint);
      if (Array.isArray(data)) {
        return data;
      }

      if (Array.isArray(data?.invoices)) {
        return data.invoices;
      }

      if (Array.isArray(data?.data)) {
        return data.data;
      }

      if (Array.isArray(data?.items)) {
        return data.items;
      }
    } catch {
      // continue trying the next endpoint if one is unavailable
    }
  }

  return [];
}
