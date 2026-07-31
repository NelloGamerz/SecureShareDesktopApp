import { useQuery } from '@tanstack/react-query';
import { fetchDashboard, type DashboardData } from './dashboard-api';

export function useDashboard() {
  return useQuery<DashboardData>({
    queryKey: ['dashboard'],
    queryFn: fetchDashboard,
    staleTime: 30_000,
  });
}
