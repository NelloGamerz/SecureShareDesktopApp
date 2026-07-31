import { zodResolver } from '@hookform/resolvers/zod';
import { useForm } from 'react-hook-form';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form';
import { Input } from '@/components/ui/input';
import { renameDeviceSchema, type RenameDeviceFormValues } from './devices-schemas';
import type { Device } from './devices-api';
import { useRemoveDevice, useRenameDevice } from './devices-hooks';

// --- Rename Device Dialog ---

export function RenameDeviceDialog({
  open,
  onOpenChange,
  device,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  device: Device | null;
}) {
  const mutation = useRenameDevice();

  const form = useForm<RenameDeviceFormValues>({
    resolver: zodResolver(renameDeviceSchema),
    values: device ? { name: device.deviceName } : undefined,
  });

  const onSubmit = (values: RenameDeviceFormValues) => {
    if (!device) return;
    mutation.mutate(
      { id: device.id, name: values.name },
      {
        onSuccess: () => {
          toast.success(`Device renamed to "${values.name}".`);
          onOpenChange(false);
        },
        onError: (err) => toast.error(err.message),
      }
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Rename device</DialogTitle>
          <DialogDescription>Give this device a more recognizable name.</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Device name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
                Save name
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

// --- Remove Device Dialog ---

export function RemoveDeviceDialog({
  open,
  onOpenChange,
  device,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  device: Device | null;
}) {
  const mutation = useRemoveDevice();

  const confirm = () => {
    if (!device) return;
    mutation.mutate(device.id, {
      onSuccess: () => {
        toast.success(`${device.deviceName} has been removed.`);
        onOpenChange(false);
      },
      onError: (err) => toast.error(err.message),
    });
  };

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove device?</AlertDialogTitle>
          <AlertDialogDescription>
            This will revoke access for "{device?.deviceName ?? 'this device'}" and remove it from
            your workspace. This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            onClick={confirm}
            disabled={mutation.isPending}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            {mutation.isPending ? 'Removing…' : 'Remove device'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
