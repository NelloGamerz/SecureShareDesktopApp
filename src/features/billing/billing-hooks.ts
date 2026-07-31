import { useQuery } from '@tanstack/react-query';
import { fetchBillingInvoices, fetchBillingUsageSummary } from './billing-api';

export function useBillingUsageSummary() {
  return useQuery({
    queryKey: ['billing', 'usage-summary'],
    queryFn: fetchBillingUsageSummary,
    staleTime: 30_000,
  });
}

export function useBillingInvoices() {
  return useQuery({
    queryKey: ['billing', 'invoices'],
    queryFn: fetchBillingInvoices,
    staleTime: 30_000,
  });
}
