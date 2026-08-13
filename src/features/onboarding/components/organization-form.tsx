import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  organizationSchema,
  slugify,
  type OrganizationFormValues,
} from '../onboarding-schemas';
import type { Industry, OrganizationSize } from '../onboarding-types';

interface OrganizationFormProps {
  defaults?: Partial<OrganizationFormValues>;
  onChange: (values: OrganizationFormValues | null) => void;
}

const sizes: OrganizationSize[] = ['1-5', '6-20', '21-50', '51-100', '100+'];
const industries: { value: Industry; label: string }[] = [
  { value: 'SOFTWARE', label: 'Software' },
  { value: 'VIDEO_EDITING', label: 'Video Editing' },
  { value: 'MARKETING', label: 'Marketing' },
  { value: 'EDUCATION', label: 'Education' },
  { value: 'AGENCY', label: 'Agency' },
  { value: 'MANUFACTURING', label: 'Manufacturing' },
  { value: 'OTHER', label: 'Other' },
];

/** Organization details form — name, size, industry, slug (with auto-slug from name). */
export function OrganizationForm({ defaults, onChange }: OrganizationFormProps) {
  const form = useForm<OrganizationFormValues>({
    resolver: zodResolver(organizationSchema),
    defaultValues: {
      organizationName: defaults?.organizationName ?? '',
      organizationSize: (defaults?.organizationSize ?? '') as OrganizationFormValues['organizationSize'],
      industry: (defaults?.industry ?? '') as OrganizationFormValues['industry'],
      workspaceSlug: defaults?.workspaceSlug ?? '',
    },
    mode: 'onChange',
  });

  const values = form.watch();
  const isValid = form.formState.isValid;

  // Report validity + values up to the parent on every change.
  useEffect(() => {
    onChange(isValid ? values : null);
  }, [values, isValid, onChange]);

  // Auto-generate slug from the organization name unless the user has
  // manually edited the slug.
  const orgName = values.organizationName;
  const slugValue = values.workspaceSlug ?? '';

  useEffect(() => {
    if (!orgName) return;
    const generated = slugify(orgName);
    // Only overwrite when the current slug is empty or matches the
    // previously-generated slug (i.e. the user hasn't manually edited it).
    const previousFromName = slugify(orgName.slice(0, -1));
    if (!slugValue || slugValue === previousFromName || slugValue === generated) {
      form.setValue('workspaceSlug', generated, { shouldValidate: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orgName]);

  return (
    <Form {...form}>
      <form className="space-y-5" onSubmit={(e) => e.preventDefault()}>
        <FormField
          control={form.control}
          name="organizationName"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Organization name</FormLabel>
              <FormControl>
                <Input placeholder="Acme Technologies" autoComplete="off" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        <div className="grid gap-4 sm:grid-cols-2">
          <FormField
            control={form.control}
            name="organizationSize"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Organization size</FormLabel>
                <Select onValueChange={field.onChange} value={field.value}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue placeholder="Select size" />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {sizes.map((s) => (
                      <SelectItem key={s} value={s}>
                        {s}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />

          <FormField
            control={form.control}
            name="industry"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Industry</FormLabel>
                <Select onValueChange={field.onChange} value={field.value}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue placeholder="Select industry" />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {industries.map((i) => (
                      <SelectItem key={i.value} value={i.value}>
                        {i.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
        </div>

        {/* <FormField
          control={form.control}
          name="workspaceSlug"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Workspace slug (optional)</FormLabel>
              <div className="flex items-center rounded-md border border-input bg-background focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-1">
                <span className="border-r bg-muted px-3 py-2 text-sm text-muted-foreground">
                  secureshare.app/
                </span>
                <input
                  {...field}
                  className="flex-1 bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground"
                  placeholder="acme"
                />
              </div>
              <p className="text-xs text-muted-foreground">
                Lowercase letters, numbers, and hyphens only.
              </p>
              <FormMessage />
            </FormItem>
          )}
        /> */}
      </form>
    </Form>
  );
}
