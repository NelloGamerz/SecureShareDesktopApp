import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import {
  cancelPairing,
  checkPairingStatus,
  connectPairing,
  createPairing,
  fetchDeviceDetail,
  fetchDeviceHealth,
  fetchDevices,
  registerDevice,
  removeDevice,
  renameDevice,
  type ConnectPairingInput,
  type CreatePairingInput,
  type Device,
  type DeviceHealthSummary,
  type OrganizationMember,
} from './devices-api';

const DEVICES_KEY = ['devices'];
const DEVICE_HEALTH_KEY = ['device-health'];
const MY_DEVICES_KEY = ['my-devices'];
const ORGANIZATION_MEMBERS_KEY = ['organization-members'];
const deviceDetailKey = (id: string) => ['device', id];

export function useDevices() {
  return useQuery({
    queryKey: DEVICES_KEY,
    queryFn: fetchDevices,
    staleTime: 30_000,
  });
}

export function useDeviceHealth() {
  return useQuery({
    queryKey: DEVICE_HEALTH_KEY,
    queryFn: fetchDeviceHealth,
    staleTime: 30_000,
  });
}

export function useMyDevices() {
  return useQuery({
    queryKey: MY_DEVICES_KEY,
    queryFn: async () => {
      const { data } = await api.get<Device[]>('/devices');
      return data;
    },
    staleTime: 30_000,
  });
}

export function useOrganizationMembers() {
  return useQuery({
    queryKey: ORGANIZATION_MEMBERS_KEY,
    queryFn: async () => {
      const { data } = await api.get<OrganizationMember[]>('/members');
      return data;
    },
    staleTime: 30_000,
  });
}

export function useDeviceDetail(id: string | undefined) {
  return useQuery({
    queryKey: deviceDetailKey(id ?? ''),
    queryFn: () => fetchDeviceDetail(id!),
    enabled: Boolean(id),
    staleTime: 30_000,
  });
}

function invalidateDevices(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: DEVICES_KEY });
  qc.invalidateQueries({ queryKey: DEVICE_HEALTH_KEY });
  qc.invalidateQueries({ queryKey: ['device'] });
}

export function useRegisterDevice() {
  const qc = useQueryClient();
  return useMutation<Device, Error, Parameters<typeof registerDevice>[0]>({
    mutationFn: registerDevice,
    onSuccess: () => invalidateDevices(qc),
  });
}

export function useRenameDevice() {
  const qc = useQueryClient();
  return useMutation<Device, Error, { id: string; name: string }>({
    mutationFn: ({ id, name }) => renameDevice(id, name),
    onSuccess: () => invalidateDevices(qc),
  });
}

export function useRemoveDevice() {
  const qc = useQueryClient();
  return useMutation<void, Error, string>({
    mutationFn: removeDevice,
    onSuccess: () => invalidateDevices(qc),
  });
}

export function useCreatePairing() {
  return useMutation({
    mutationFn: (input: CreatePairingInput) => createPairing(input),
  });
}

export function useCheckPairingStatus() {
  return useMutation({
    mutationFn: (code: string) => checkPairingStatus(code),
  });
}

export function useConnectPairing() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ code, input }: { code: string; input: ConnectPairingInput }) =>
      connectPairing(code, input),
    onSuccess: () => invalidateDevices(qc),
  });
}

export function useCancelPairing() {
  return useMutation({
    mutationFn: (code: string) => cancelPairing(code),
  });
}

export type { DeviceHealthSummary };
