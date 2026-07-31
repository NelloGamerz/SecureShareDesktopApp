import { z } from 'zod';

export const registerDeviceSchema = z.object({
  name: z.string().min(2, 'Device name must be at least 2 characters'),
  type: z.enum(['laptop', 'phone', 'tablet', 'server']),
  os: z.string().min(1, 'Operating system is required'),
  region: z.string().optional(),
  ownerName: z.string().optional(),
  connectionType: z.enum(['lan', 'remote']).default('remote'),
});

export type RegisterDeviceFormValues = z.infer<typeof registerDeviceSchema>;

export const renameDeviceSchema = z.object({
  name: z.string().min(2, 'Device name must be at least 2 characters'),
});

export type RenameDeviceFormValues = z.infer<typeof renameDeviceSchema>;

export const pairingSchema = z.object({
  deviceName: z.string().optional(),
  deviceType: z.enum(['laptop', 'phone', 'tablet', 'server']).default('laptop'),
});

export type PairingFormValues = z.infer<typeof pairingSchema>;
