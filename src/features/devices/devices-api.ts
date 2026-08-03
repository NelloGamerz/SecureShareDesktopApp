import { api } from "@/lib/api";
import { DeviceType } from "../dashboard/dashboard-api";
import { getDeviceInfo } from "@/services/getDeviceInfo";
import { createDeviceIdentity } from "@/api/tauri";

// --- Types ---

export type DeviceStatus = "online" | "idle" | "offline";
export type ConnectionType = "lan" | "remote";
export type PairingStatus = "pending" | "connected" | "expired" | "cancelled";

export interface Device {
  id: string;
  deviceName: string;
  type: DeviceType;
  operatingSystem: string;
  deviceIdentifier: string;
  status: DeviceStatus;
  region: string;
  health: number;
  ownerName: string | null;
  connectionType: ConnectionType;
  currentTransfer: string | null;
  lastSeenAt: string;
  appVersion: string;
  createdAt: string;
}

export interface OrganizationMember {
  id: string;
  name: string;
  firstName: string | null;
  lastName: string | null;
  profileImage: string | null;
  devices: Device[];
}

export interface DeviceTransfer {
  id: string;
  fileName: string;
  sizeBytes: number;
  direction: "in" | "out";
  status: string;
  createdAt: string;
}

export interface DeviceDetail extends Device {
  recentTransfers: DeviceTransfer[];
}

export interface DeviceHealthSummary {
  total: number;
  online: number;
  healthy: number;
  warning: number;
  critical: number;
}

export interface PairingSession {
  id: string;
  pairingCode: string;
  status: PairingStatus;
  deviceName: string | null;
  deviceType: DeviceType;
  expiresAt: string;
  connectedDeviceId: string | null;
  createdAt: string;
  device?: Device;
}

// --- API functions ---

export async function fetchDevices(): Promise<Device[]> {
  const { data } = await api.get<Device[]>("/devices");
  return data;
}

export async function fetchDeviceHealth(): Promise<DeviceHealthSummary> {
  const { data } = await api.get<DeviceHealthSummary>("/devices/health");
  return data;
}

export async function fetchDeviceDetail(id: string): Promise<DeviceDetail> {
  const { data } = await api.get<DeviceDetail>(`/devices/${id}`);
  return data;
}

export interface RegisterDeviceInput {
  name: string;
  type: DeviceType;
  os: string;
  region?: string;
  ownerName?: string;
  connectionType?: ConnectionType;
}

export async function registerDevice(
  input: RegisterDeviceInput,
): Promise<Device> {
  const { data } = await api.post<Device>("/devices/register", input);
  return data;
}

export async function renameDevice(id: string, name: string): Promise<Device> {
  const { data } = await api.put<Device>(`/devices/${id}/rename`, { name });
  return data;
}

export async function removeDevice(id: string): Promise<void> {
  await api.delete(`/devices/${id}`);
}

export interface CreatePairingInput {
  deviceName?: string;
  deviceType: DeviceType;
}

export async function createPairing(
  input: CreatePairingInput,
): Promise<PairingSession> {
  const { data } = await api.post<PairingSession>("/devices/pair", input);
  return data;
}

export async function checkPairingStatus(
  code: string,
): Promise<PairingSession> {
  const { data } = await api.get<PairingSession>(`/devices/pair/${code}`);
  return data;
}

export interface ConnectPairingInput {
  deviceName?: string;
  os?: string;
  region?: string;
  ownerName?: string;
  connectionType?: ConnectionType;
}

export async function connectPairing(
  code: string,
  input: ConnectPairingInput,
): Promise<PairingSession> {
  const { data } = await api.post<PairingSession>(
    `/devices/pair/${code}/connect`,
    input,
  );
  return data;
}

export async function cancelPairing(code: string): Promise<PairingSession> {
  const { data } = await api.post<PairingSession>(
    `/devices/pair/${code}/cancel`,
  );
  return data;
}

export async function registerCurrentDevice() {
  const deviceInfo = await getDeviceInfo();
  const publicKey = await createDeviceIdentity(); 

  const { data } = await api.post("/devices/register", {
    ...deviceInfo,
    publicKey,
  });

  return data;
}
