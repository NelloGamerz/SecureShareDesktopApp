import { api } from '@/lib/api';

export type TransferStatus = 'completed' | 'in_progress' | 'queued' | 'failed';
export type TransferDirection = 'in' | 'out';
export type DeviceStatus = 'online' | 'idle' | 'offline';
export type DeviceType = 'laptop' | 'phone' | 'tablet' | 'server' | 'desktop';
export type ActivityType = 'transfer' | 'device' | 'member' | 'system';

export interface DashboardStats {
  activeTransfers: number;
  onlineDevices: number;
  totalDevices: number;
  totalMembers: number;
  storageUsedTB: number;
  storageTotalTB: number;
  storagePercent: number;
}

export interface TransferVolumePoint {
  day: string;
  sent: number;
  received: number;
}

export interface DevicesByRegionPoint {
  name: string;
  value: number;
}

export interface RecentTransfer {
  id: string;
  fileName: string;
  sizeBytes: number;
  direction: TransferDirection;
  recipient: string;
  status: TransferStatus;
  createdAt: string;
}

export interface ActivityEvent {
  id: string;
  type: ActivityType;
  title: string;
  description: string;
  timestamp: string;
}

export interface DeviceHealth {
  id: string;
  name: string;
  type: DeviceType;
  os: string;
  status: DeviceStatus;
  region: string;
  health: number;
  lastSeenAt: string;
}

export interface HealthSummary {
  healthy: number;
  warning: number;
  critical: number;
}

export interface DashboardData {
  stats: DashboardStats;
  transferVolume: TransferVolumePoint[];
  devicesByRegion: DevicesByRegionPoint[];
  recentTransfers: RecentTransfer[];
  recentActivity: ActivityEvent[];
  deviceHealth: DeviceHealth[];
  healthSummary: HealthSummary;
}

export async function fetchDashboard(): Promise<DashboardData> {
  const { data } = await api.get<DashboardData>('/dashboard');
  return data;
}
