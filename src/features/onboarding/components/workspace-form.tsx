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
import { individualSchema, type IndividualFormValues } from '../onboarding-schemas';

interface WorkspaceFormProps {
  prefill: string;
  onChange: (values: IndividualFormValues | null) => void;
}

/** Individual workspace form — single "Workspace Name" field. */
export function WorkspaceForm({ prefill, onChange }: WorkspaceFormProps) {
  const form = useForm<IndividualFormValues>({
    resolver: zodResolver(individualSchema),
    defaultValues: { workspaceName: prefill },
    mode: 'onChange',
  });

  const values = form.watch();
  const isValid = form.formState.isValid;

  // Report validity + values up to the parent on every change.
  useEffect(() => {
    onChange(isValid ? values : null);
  }, [values, isValid, onChange]);

  return (
    <Form {...form}>
      <form className="space-y-5" onSubmit={(e) => e.preventDefault()}>
        <FormField
          control={form.control}
          name="workspaceName"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Workspace name</FormLabel>
              <FormControl>
                <Input placeholder="Karan's Workspace" autoComplete="off" {...field} />
              </FormControl>
              <p className="text-xs text-muted-foreground">You can change this later.</p>
              <FormMessage />
            </FormItem>
          )}
        />
      </form>
    </Form>
  );
}
