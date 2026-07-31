// import { zodResolver } from '@hookform/resolvers/zod';
// import { motion } from 'framer-motion';
// import { ArrowLeft, HardDrive, Laptop, Loader2, Server, Smartphone, Tablet } from 'lucide-react';
// import { useForm } from 'react-hook-form';
// import { Link, useNavigate } from 'react-router-dom';
// import { toast } from 'sonner';
// import { Button } from '@/components/ui/button';
// import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
// import {
//   Form,
//   FormControl,
//   FormField,
//   FormItem,
//   FormLabel,
//   FormMessage,
// } from '@/components/ui/form';
// import { Input } from '@/components/ui/input';
// import {
//   Select,
//   SelectContent,
//   SelectItem,
//   SelectTrigger,
//   SelectValue,
// } from '@/components/ui/select';
// import { PageContainer } from '@/components/layout/page-header';
// import {
//   registerDeviceSchema,
//   type RegisterDeviceFormValues,
// } from './devices-schemas';
// import { useRegisterDevice } from './devices-hooks';
// import type { DeviceType } from './devices-api';

// const deviceTypes: { value: DeviceType; label: string; icon: typeof Laptop }[] = [
//   { value: 'laptop', label: 'Desktop / Laptop', icon: Laptop },
//   { value: 'phone', label: 'Mobile phone', icon: Smartphone },
//   { value: 'tablet', label: 'Tablet', icon: Tablet },
//   { value: 'server', label: 'Server', icon: Server },
// ];

// export function RegisterDevicePage() {
//   const navigate = useNavigate();
//   const mutation = useRegisterDevice();

//   const form = useForm<RegisterDeviceFormValues>({
//     resolver: zodResolver(registerDeviceSchema),
//     defaultValues: {
//       name: '',
//       type: 'laptop',
//       os: '',
//       region: 'US-East',
//       ownerName: '',
//       connectionType: 'remote',
//     },
//   });

//   const onSubmit = (values: RegisterDeviceFormValues) => {
//     mutation.mutate(values, {
//       onSuccess: (device) => {
//         toast.success(`${device.name} has been registered.`);
//         navigate('/devices');
//       },
//       onError: (err) => toast.error(err.message),
//     });
//   };

//   const watchedType = form.watch('type');

//   return (
//     <PageContainer>
//       <motion.div initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
//         <Link
//           to="/devices"
//           className="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
//         >
//           <ArrowLeft className="h-4 w-4" />
//           Back to devices
//         </Link>

//         <div className="mx-auto max-w-2xl">
//           <div className="mb-8 flex items-center gap-3">
//             <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary text-primary-foreground">
//               <HardDrive className="h-6 w-6" />
//             </div>
//             <div>
//               <h1 className="text-2xl font-semibold tracking-tight">Register device</h1>
//               <p className="text-sm text-muted-foreground">
//                 Manually add a device to your workspace.
//               </p>
//             </div>
//           </div>

//           <Card>
//             <CardHeader>
//               <CardTitle className="text-base">Device details</CardTitle>
//               <CardDescription>Provide information about the device you're registering.</CardDescription>
//             </CardHeader>
//             <CardContent>
//               <Form {...form}>
//                 <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-5">
//                   {/* Device type selector */}
//                   <FormField
//                     control={form.control}
//                     name="type"
//                     render={({ field }) => (
//                       <FormItem>
//                         <FormLabel>Device type</FormLabel>
//                         <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
//                           {deviceTypes.map((t) => (
//                             <button
//                               key={t.value}
//                               type="button"
//                               onClick={() => field.onChange(t.value)}
//                               className={`flex flex-col items-center gap-2 rounded-lg border p-4 transition-all ${
//                                 watchedType === t.value
//                                   ? 'border-primary bg-primary/5'
//                                   : 'hover:border-foreground/20'
//                               }`}
//                             >
//                               <t.icon className={`h-6 w-6 ${watchedType === t.value ? 'text-primary' : 'text-muted-foreground'}`} />
//                               <span className="text-xs font-medium">{t.label}</span>
//                             </button>
//                           ))}
//                         </div>
//                         <FormMessage />
//                       </FormItem>
//                     )}
//                   />

//                   <FormField
//                     control={form.control}
//                     name="name"
//                     render={({ field }) => (
//                       <FormItem>
//                         <FormLabel>Device name</FormLabel>
//                         <FormControl>
//                           <Input placeholder="Jordan's MacBook Pro" {...field} />
//                         </FormControl>
//                         <FormMessage />
//                       </FormItem>
//                     )}
//                   />

//                   <div className="grid gap-4 sm:grid-cols-2">
//                     <FormField
//                       control={form.control}
//                       name="os"
//                       render={({ field }) => (
//                         <FormItem>
//                           <FormLabel>Operating system</FormLabel>
//                           <FormControl>
//                             <Input placeholder="macOS 15.2" {...field} />
//                           </FormControl>
//                           <FormMessage />
//                         </FormItem>
//                       )}
//                     />
//                     <FormField
//                       control={form.control}
//                       name="ownerName"
//                       render={({ field }) => (
//                         <FormItem>
//                           <FormLabel>Owner (optional)</FormLabel>
//                           <FormControl>
//                             <Input placeholder="Jordan Avery" {...field} />
//                           </FormControl>
//                           <FormMessage />
//                         </FormItem>
//                       )}
//                     />
//                   </div>

//                   <div className="grid gap-4 sm:grid-cols-2">
//                     <FormField
//                       control={form.control}
//                       name="region"
//                       render={({ field }) => (
//                         <FormItem>
//                           <FormLabel>Region</FormLabel>
//                           <Select onValueChange={field.onChange} value={field.value}>
//                             <FormControl>
//                               <SelectTrigger>
//                                 <SelectValue placeholder="Select region" />
//                               </SelectTrigger>
//                             </FormControl>
//                             <SelectContent>
//                               <SelectItem value="US-East">US-East</SelectItem>
//                               <SelectItem value="US-West">US-West</SelectItem>
//                               <SelectItem value="EU">EU</SelectItem>
//                               <SelectItem value="APAC">APAC</SelectItem>
//                               <SelectItem value="Other">Other</SelectItem>
//                             </SelectContent>
//                           </Select>
//                           <FormMessage />
//                         </FormItem>
//                       )}
//                     />
//                     <FormField
//                       control={form.control}
//                       name="connectionType"
//                       render={({ field }) => (
//                         <FormItem>
//                           <FormLabel>Connection type</FormLabel>
//                           <Select onValueChange={field.onChange} value={field.value}>
//                             <FormControl>
//                               <SelectTrigger>
//                                 <SelectValue placeholder="Select connection" />
//                               </SelectTrigger>
//                             </FormControl>
//                             <SelectContent>
//                               <SelectItem value="lan">LAN (local network)</SelectItem>
//                               <SelectItem value="remote">Remote (internet)</SelectItem>
//                             </SelectContent>
//                           </Select>
//                           <FormMessage />
//                         </FormItem>
//                       )}
//                     />
//                   </div>

//                   <div className="flex items-center justify-end gap-3 border-t pt-5">
//                     <Button type="button" variant="outline" asChild>
//                       <Link to="/devices">Cancel</Link>
//                     </Button>
//                     <Button type="submit" disabled={mutation.isPending}>
//                       {mutation.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
//                       Register device
//                     </Button>
//                   </div>
//                 </form>
//               </Form>
//             </CardContent>
//           </Card>
//         </div>
//       </motion.div>
//     </PageContainer>
//   );
// }
