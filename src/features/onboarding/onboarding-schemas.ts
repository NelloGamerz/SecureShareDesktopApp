import { z } from 'zod';

// --- Step 1: usage type ---
export const usageTypeSchema = z.object({
  organizationType: z.enum(['INDIVIDUAL', 'ORGANIZATION']),
});

// --- Step 2: Individual ---
export const individualSchema = z.object({
  workspaceName: z
    .string()
    .min(3, 'Workspace name must be at least 3 characters')
    .max(100, 'Workspace name must be at most 100 characters'),
});

export type IndividualFormValues = z.infer<typeof individualSchema>;

// --- Step 2: Organization ---
export const organizationSchema = z.object({
  organizationName: z
    .string()
    .min(3, 'Organization name must be at least 3 characters')
    .max(100, 'Organization name must be at most 100 characters'),
  organizationSize: z.enum(['1-5', '6-20', '21-50', '51-100', '100+'], {
    errorMap: () => ({ message: 'Select an organization size' }),
  }),
  industry: z.enum(
    ['SOFTWARE', 'VIDEO_EDITING', 'MARKETING', 'EDUCATION', 'AGENCY', 'MANUFACTURING', 'OTHER'],
    { errorMap: () => ({ message: 'Select an industry' }) }
  ),
  workspaceSlug: z
    .string()
    .regex(/^[a-z0-9-]*$/, 'Slug must be lowercase letters, numbers, and hyphens only')
    .max(50, 'Slug must be at most 50 characters')
    .optional()
    .or(z.literal('')),
});

export type OrganizationFormValues = z.infer<typeof organizationSchema>;

// --- Step 3: Invites ---
export const emailSchema = z.string().email('Enter a valid email address');

export const invitesSchema = z.object({
  invites: z.array(emailSchema).max(25, 'You can invite up to 25 people at once'),
});

export type InvitesFormValues = z.infer<typeof invitesSchema>;

// --- Final: terms agreement ---
export const termsSchema = z.object({
  agreedToTerms: z.literal(true, {
    errorMap: () => ({ message: 'You must agree to the Terms and Privacy Policy' }),
  }),
});

// --- Slug generation helper ---
export function slugify(input: string): string {
  return input
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9\s-]/g, '')
    .replace(/[\s_]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-+|-+$/g, '');
}
