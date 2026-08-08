import { useQuery } from '@tanstack/react-query';
import { fetchBillingInvoices, fetchBillingUsageSummary } from './billing-api';

export function useBillingUsageSummary() {
  return useQuery({
    queryKey: ['billing', 'usage-summary'],
    queryFn: fetchBillingUsageSummary,
    staleTime: 30_000,
  });
}

export function useBillingInvoices(page = 0, size = 10) {
  return useQuery({
    queryKey: ['billing-invoices', page, size],
    queryFn: () => fetchBillingInvoices(page, size),
  });
}
