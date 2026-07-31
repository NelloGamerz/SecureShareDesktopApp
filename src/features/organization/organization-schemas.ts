import { z } from 'zod';

export const organizationSchema = z.object({
  name: z.string().min(2, 'Organization name must be at least 2 characters'),
  website: z
    .string()
    .url('Enter a valid URL (e.g. https://example.com)')
    .or(z.literal(''))
    .optional(),
  industry: z.string().min(1, 'Industry is required').or(z.literal('')).optional(),
  teamSize: z.coerce.number().int().min(0, 'Team size cannot be negative').max(100000, 'That seems too large'),
  contactName: z.string().min(1, 'Contact name is required').or(z.literal('')).optional(),
  contactEmail: z
    .string()
    .email('Enter a valid email')
    .or(z.literal(''))
    .optional(),
  contactPhone: z.string().or(z.literal('')).optional(),
  city: z.string().or(z.literal('')).optional(),
  country: z.string().or(z.literal('')).optional(),
});

export type OrganizationFormValues = z.infer<typeof organizationSchema>;

export const inviteSchema = z.object({
  email: z.string().email('Enter a valid email address'),
  role: z.enum(['OWNER', 'ADMIN', 'MEMBER']),
});

export type InviteFormValues = z.infer<typeof inviteSchema>;

export const roleSchema = z.object({
  role: z.enum(['OWNER', 'ADMIN', 'MEMBER']),
});

export type RoleFormValues = z.infer<typeof roleSchema>;
