// import { motion } from 'framer-motion';
// import {
//   ArrowLeft,
//   CheckCircle2,
//   Clock,
//   Loader2,
//   QrCode,
//   RefreshCw,
//   Smartphone,
//   Tablet,
//   Laptop,
//   Server,
//   Wifi,
//   WifiOff,
//   XCircle,
//   Zap,
// } from 'lucide-react';
// import QRCode from 'qrcode';
// import { useEffect, useRef, useState } from 'react';
// import { Link, useNavigate } from 'react-router-dom';
// import { toast } from 'sonner';
// import { Button } from '@/components/ui/button';
// import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
// import {
//   Select,
//   SelectContent,
//   SelectItem,
//   SelectTrigger,
//   SelectValue,
// } from '@/components/ui/select';
// import { Label } from '@/components/ui/label';
// import { PageContainer } from '@/components/layout/page-header';
// import {
//   useCancelPairing,
//   useCheckPairingStatus,
//   useConnectPairing,
//   useCreatePairing,
// } from './devices-hooks';
// import type { PairingSession } from './devices-api';
// import { env } from '@/lib/env';

// const deviceTypeOptions = [
//   { value: 'laptop', label: 'Desktop / Laptop', icon: Laptop },
//   { value: 'phone', label: 'Mobile phone', icon: Smartphone },
//   { value: 'tablet', label: 'Tablet', icon: Tablet },
//   { value: 'server', label: 'Server', icon: Server },
// ];

// function useCountdown(expiresAt: string | null, active: boolean) {
//   const [remaining, setRemaining] = useState(0);

//   useEffect(() => {
//     if (!expiresAt || !active) return;
//     const update = () => Math.max(0, new Date(expiresAt).getTime() - Date.now());
//     setRemaining(update());
//     const interval = setInterval(() => {
//       const r = update();
//       setRemaining(r);
//       if (r <= 0) clearInterval(interval);
//     }, 1000);
//     return () => clearInterval(interval);
//   }, [expiresAt, active]);

//   const minutes = Math.floor(remaining / 60000);
//   const seconds = Math.floor((remaining % 60000) / 1000);
//   return {
//     remaining,
//     display: `${minutes}:${seconds.toString().padStart(2, '0')}`,
//     expired: remaining <= 0,
//   };
// }

// export function QrPairingPage() {
//   const navigate = useNavigate();
//   const createMutation = useCreatePairing();
//   const checkMutation = useCheckPairingStatus();
//   const connectMutation = useConnectPairing();
//   const cancelMutation = useCancelPairing();

//   const [deviceType, setDeviceType] = useState('laptop');
//   const [session, setSession] = useState<PairingSession | null>(null);
//   const [qrDataUrl, setQrDataUrl] = useState<string>('');
//   const [wsConnected, setWsConnected] = useState(false);
//   const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

//   const countdown = useCountdown(session?.expiresAt ?? null, session?.status === 'pending');

//   // Generate QR code when session changes
//   useEffect(() => {
//     if (!session) return;
//     const payload = JSON.stringify({
//       code: session.pairingCode,
//       server: env.supabaseUrl,
//       type: session.deviceType,
//     });
//     QRCode.toDataURL(payload, {
//       width: 280,
//       margin: 2,
//       color: { dark: '#0a0a0b', light: '#ffffff' },
//       errorCorrectionLevel: 'M',
//     })
//       .then(setQrDataUrl)
//       .catch(() => setQrDataUrl(''));
//   }, [session?.pairingCode]);

//   // Simulate WebSocket connection + poll for status updates
//   useEffect(() => {
//     if (!session || session.status !== 'pending') return;

//     // Simulate WS connection establishment
//     const wsTimer = setTimeout(() => setWsConnected(true), 800);

//     // Poll the pairing status every 2 seconds (simulating WS events)
//     pollRef.current = setInterval(async () => {
//       try {
//         const updated = await checkMutation.mutateAsync(session.pairingCode);
//         setSession(updated);
//         if (updated.status === 'connected') {
//           if (pollRef.current) clearInterval(pollRef.current);
//           toast.success('Device connected successfully!');
//         } else if (updated.status === 'expired' || updated.status === 'cancelled') {
//           if (pollRef.current) clearInterval(pollRef.current);
//           setWsConnected(false);
//         }
//       } catch {
//         // keep polling
//       }
//     }, 2000);

//     return () => {
//       clearTimeout(wsTimer);
//       if (pollRef.current) clearInterval(pollRef.current);
//       setWsConnected(false);
//     };
//     // eslint-disable-next-line react-hooks/exhaustive-deps
//   }, [session?.pairingCode, session?.status]);

//   const handleCreate = () => {
//     createMutation.mutate(
//       { deviceType: deviceType as PairingSession['deviceType'] },
//       {
//         onSuccess: (s) => {
//           setSession(s);
//           setWsConnected(false);
//         },
//         onError: (err) => toast.error(err.message),
//       }
//     );
//   };

//   const handleSimulateConnect = () => {
//     if (!session) return;
//     connectMutation.mutate(
//       {
//         code: session.pairingCode,
//         input: {
//           deviceName: session.deviceName ?? `Paired ${deviceType}`,
//           os: 'Auto-detected',
//           region: 'US-East',
//           connectionType: 'remote',
//         },
//       },
//       {
//         onSuccess: (s) => {
//           setSession(s);
//           if (pollRef.current) clearInterval(pollRef.current);
//         },
//         onError: (err) => toast.error(err.message),
//       }
//     );
//   };

//   const handleCancel = () => {
//     if (!session) return;
//     cancelMutation.mutate(session.pairingCode, {
//       onSuccess: (s) => {
//         setSession(s);
//         if (pollRef.current) clearInterval(pollRef.current);
//         setWsConnected(false);
//       },
//       onError: (err) => toast.error(err.message),
//     });
//   };

//   const handleReset = () => {
//     setSession(null);
//     setQrDataUrl('');
//     setWsConnected(false);
//     if (pollRef.current) clearInterval(pollRef.current);
//   };

//   const status = session?.status ?? 'pending';
//   const isPending = status === 'pending' && !countdown.expired;
//   const isExpired = status === 'expired' || (status === 'pending' && countdown.expired);
//   const isConnected = status === 'connected';

//   return (
//     <PageContainer>
//       <Link
//         to="/devices"
//         className="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
//       >
//         <ArrowLeft className="h-4 w-4" />
//         Back to devices
//       </Link>

//       <motion.div initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
//         <div className="mb-8 flex items-center gap-3">
//           <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary text-primary-foreground">
//             <QrCode className="h-6 w-6" />
//           </div>
//           <div>
//             <h1 className="text-2xl font-semibold tracking-tight">QR Pairing</h1>
//             <p className="text-sm text-muted-foreground">
//               Scan the QR code with a device to instantly pair it with your workspace.
//             </p>
//           </div>
//         </div>
//       </motion.div>

//       <div className="mx-auto max-w-3xl">
//         <div className="grid gap-6 lg:grid-cols-2">
//           {/* QR code panel */}
//           <Card>
//             <CardHeader>
//               <CardTitle className="text-base">Pairing code</CardTitle>
//               <CardDescription>
//                 {session
//                   ? 'Scan with your device camera or pairing app'
//                   : 'Generate a code to start pairing'}
//               </CardDescription>
//             </CardHeader>
//             <CardContent className="flex flex-col items-center gap-4">
//               {!session && (
//                 <>
//                   <div className="flex h-[280px] w-full items-center justify-center rounded-lg border-2 border-dashed bg-muted/30">
//                     <QrCode className="h-16 w-16 text-muted-foreground/30" />
//                   </div>
//                   <div className="w-full space-y-3">
//                     <div className="space-y-2">
//                       <Label>Device type</Label>
//                       <Select value={deviceType} onValueChange={setDeviceType}>
//                         <SelectTrigger className="w-full">
//                           <SelectValue />
//                         </SelectTrigger>
//                         <SelectContent>
//                           {deviceTypeOptions.map((t) => (
//                             <SelectItem key={t.value} value={t.value}>
//                               {t.label}
//                             </SelectItem>
//                           ))}
//                         </SelectContent>
//                       </Select>
//                     </div>
//                     <Button
//                       className="w-full"
//                       onClick={handleCreate}
//                       disabled={createMutation.isPending}
//                     >
//                       {createMutation.isPending ? (
//                         <Loader2 className="h-4 w-4 animate-spin" />
//                       ) : (
//                         <QrCode className="h-4 w-4" />
//                       )}
//                       Generate pairing code
//                     </Button>
//                   </div>
//                 </>
//               )}

//               {session && (
//                 <>
//                   <motion.div
//                     initial={{ opacity: 0, scale: 0.9 }}
//                     animate={{ opacity: 1, scale: 1 }}
//                     transition={{ type: 'spring', stiffness: 300, damping: 25 }}
//                     className={`relative rounded-lg border-2 bg-white p-4 ${
//                       isExpired ? 'opacity-40' : ''
//                     } ${isConnected ? 'border-success' : 'border-border'}`}
//                   >
//                     {qrDataUrl ? (
//                       <img src={qrDataUrl} alt="QR pairing code" className="h-[240px] w-[240px]" />
//                     ) : (
//                       <div className="flex h-[240px] w-[240px] items-center justify-center">
//                         <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
//                       </div>
//                     )}
//                     {isConnected && (
//                       <motion.div
//                         initial={{ scale: 0 }}
//                         animate={{ scale: 1 }}
//                         className="absolute inset-0 flex items-center justify-center rounded-lg bg-success/10 backdrop-blur-sm"
//                       >
//                         <CheckCircle2 className="h-16 w-16 text-success" />
//                       </motion.div>
//                     )}
//                     {isExpired && (
//                       <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-background/80 backdrop-blur-sm">
//                         <XCircle className="h-16 w-16 text-muted-foreground" />
//                       </div>
//                     )}
//                   </motion.div>

//                   {/* Pairing code text */}
//                   <div className="rounded-lg border bg-muted/30 px-4 py-2.5">
//                     <p className="font-mono text-sm font-semibold tracking-widest">
//                       {session.pairingCode}
//                     </p>
//                   </div>

//                   {/* Action buttons */}
//                   <div className="flex w-full gap-2">
//                     {isPending && (
//                       <Button
//                         variant="outline"
//                         size="sm"
//                         className="flex-1"
//                         onClick={handleCancel}
//                         disabled={cancelMutation.isPending}
//                       >
//                         Cancel
//                       </Button>
//                     )}
//                     {(isExpired || isConnected) && (
//                       <Button
//                         variant="outline"
//                         size="sm"
//                         className="flex-1"
//                         onClick={handleReset}
//                       >
//                         <RefreshCw className="h-4 w-4" />
//                         {isConnected ? 'Pair another device' : 'Generate new code'}
//                       </Button>
//                     )}
//                     {isConnected && session.connectedDeviceId && (
//                       <Button
//                         size="sm"
//                         className="flex-1"
//                         onClick={() => navigate(`/devices/${session.connectedDeviceId}`)}
//                       >
//                         View device
//                       </Button>
//                     )}
//                   </div>
//                 </>
//               )}
//             </CardContent>
//           </Card>

//           {/* Status panel */}
//           <div className="space-y-4">
//             {/* Pairing status */}
//             <Card>
//               <CardHeader>
//                 <CardTitle className="text-base">Pairing status</CardTitle>
//               </CardHeader>
//               <CardContent className="space-y-4">
//                 <StatusRow
//                   icon={isPending ? Loader2 : isConnected ? CheckCircle2 : isExpired ? XCircle : Clock}
//                   iconClass={
//                     isPending
//                       ? 'animate-spin text-warning'
//                       : isConnected
//                       ? 'text-success'
//                       : isExpired
//                       ? 'text-destructive'
//                       : 'text-muted-foreground'
//                   }
//                   label="Status"
//                   value={
//                     isConnected ? 'Connected' :
//                     isExpired ? 'Expired' :
//                     session?.status === 'cancelled' ? 'Cancelled' :
//                     isPending ? 'Waiting for device…' : 'Not started'
//                   }
//                 />

//                 {session && (
//                   <>
//                     <StatusRow
//                       icon={Clock}
//                       label="Time remaining"
//                       value={countdown.display}
//                       accent={countdown.expired ? 'text-destructive' : undefined}
//                     />
//                     <StatusRow
//                       icon={Wifi}
//                       label="WebSocket"
//                       value={wsConnected ? 'Connected' : 'Connecting…'}
//                       iconClass={wsConnected ? 'text-success' : 'text-warning'}
//                     />
//                     <StatusRow
//                       icon={isPending ? Wifi : WifiOff}
//                       label="Connection"
//                       value={isConnected ? 'Established' : isPending ? 'Waiting' : 'No connection'}
//                       iconClass={isConnected ? 'text-success' : ''}
//                     />
//                   </>
//                 )}
//               </CardContent>
//             </Card>

//             {/* Instructions */}
//             <Card>
//               <CardHeader>
//                 <CardTitle className="text-base">How to pair</CardTitle>
//               </CardHeader>
//               <CardContent className="space-y-3">
//                 {[
//                   'Open the Helix app on the device you want to pair',
//                   'Go to Settings and tap "Pair new workspace"',
//                   'Point the camera at this QR code',
//                   'Confirm the connection on both devices',
//                 ].map((step, i) => (
//                   <div key={i} className="flex items-start gap-3">
//                     <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold">
//                       {i + 1}
//                     </div>
//                     <p className="pt-0.5 text-sm text-muted-foreground">{step}</p>
//                   </div>
//                 ))}
//               </CardContent>
//             </Card>

//             {/* Simulate connection button (for testing) */}
//             {isPending && (
//               <Card className="border-dashed">
//                 <CardContent className="flex items-center justify-between p-4">
//                   <div>
//                     <p className="text-sm font-medium">Simulate connection</p>
//                     <p className="text-xs text-muted-foreground">For testing the pairing flow.</p>
//                   </div>
//                   <Button
//                     variant="outline"
//                     size="sm"
//                     onClick={handleSimulateConnect}
//                     disabled={connectMutation.isPending}
//                   >
//                     <Zap className="h-4 w-4" />
//                     Connect
//                   </Button>
//                 </CardContent>
//               </Card>
//             )}
//           </div>
//         </div>
//       </div>
//     </PageContainer>
//   );
// }

// function StatusRow({
//   icon: Icon,
//   iconClass,
//   label,
//   value,
//   accent,
// }: {
//   icon: typeof Clock;
//   iconClass?: string;
//   label: string;
//   value: string;
//   accent?: string;
// }) {
//   return (
//     <div className="flex items-center justify-between">
//       <div className="flex items-center gap-2.5">
//         <Icon className={`h-4 w-4 ${iconClass ?? 'text-muted-foreground'}`} />
//         <span className="text-sm text-muted-foreground">{label}</span>
//       </div>
//       <span className={`text-sm font-medium ${accent ?? ''}`}>{value}</span>
//     </div>
//   );
// }


import { motion } from 'framer-motion';
import {
  ArrowLeft,
  CheckCircle2,
  Clock,
  Loader2,
  QrCode,
  RefreshCw,
  Smartphone,
  Tablet,
  Laptop,
  Server,
  Wifi,
  WifiOff,
  XCircle,
  Zap,
} from 'lucide-react';
import QRCode from 'qrcode';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Label } from '@/components/ui/label';
import { PageContainer } from '@/components/layout/page-header';
import {
  useCancelPairing,
  useCheckPairingStatus,
  useConnectPairing,
  useCreatePairing,
} from './devices-hooks';
import type { PairingSession } from './devices-api';
import { env } from '@/lib/env';

const deviceTypeOptions = [
  { value: 'laptop', label: 'Desktop / Laptop', icon: Laptop },
  { value: 'phone', label: 'Mobile phone', icon: Smartphone },
  { value: 'tablet', label: 'Tablet', icon: Tablet },
  { value: 'server', label: 'Server', icon: Server },
];

function useCountdown(expiresAt: string | null, active: boolean) {
  const [remaining, setRemaining] = useState(0);

  useEffect(() => {
    if (!expiresAt || !active) return;
    const update = () => Math.max(0, new Date(expiresAt).getTime() - Date.now());
    setRemaining(update());
    const interval = setInterval(() => {
      const r = update();
      setRemaining(r);
      if (r <= 0) clearInterval(interval);
    }, 1000);
    return () => clearInterval(interval);
  }, [expiresAt, active]);

  const minutes = Math.floor(remaining / 60000);
  const seconds = Math.floor((remaining % 60000) / 1000);
  return {
    remaining,
    display: `${minutes}:${seconds.toString().padStart(2, '0')}`,
    expired: remaining <= 0,
  };
}

export function QrPairingPage() {
  const navigate = useNavigate();
  const createMutation = useCreatePairing();
  const checkMutation = useCheckPairingStatus();
  const connectMutation = useConnectPairing();
  const cancelMutation = useCancelPairing();

  const [deviceType, setDeviceType] = useState('laptop');
  const [session, setSession] = useState<PairingSession | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string>('');
  const [wsConnected, setWsConnected] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const countdown = useCountdown(session?.expiresAt ?? null, session?.status === 'pending');

  // Generate QR code when session changes
  useEffect(() => {
    if (!session) return;
    const payload = JSON.stringify({
      code: session.pairingCode,
      server: env.apiBaseUrl,
      type: session.deviceType,
    });
    QRCode.toDataURL(payload, {
      width: 280,
      margin: 2,
      color: { dark: '#0a0a0b', light: '#ffffff' },
      errorCorrectionLevel: 'M',
    })
      .then(setQrDataUrl)
      .catch(() => setQrDataUrl(''));
  }, [session?.pairingCode]);

  // Simulate WebSocket connection + poll for status updates
  useEffect(() => {
    if (!session || session.status !== 'pending') return;

    // Simulate WS connection establishment
    const wsTimer = setTimeout(() => setWsConnected(true), 800);

    // Poll the pairing status every 2 seconds (simulating WS events)
    pollRef.current = setInterval(async () => {
      try {
        const updated = await checkMutation.mutateAsync(session.pairingCode);
        setSession(updated);
        if (updated.status === 'connected') {
          if (pollRef.current) clearInterval(pollRef.current);
          toast.success('Device connected successfully!');
        } else if (updated.status === 'expired' || updated.status === 'cancelled') {
          if (pollRef.current) clearInterval(pollRef.current);
          setWsConnected(false);
        }
      } catch {
        // keep polling
      }
    }, 2000);

    return () => {
      clearTimeout(wsTimer);
      if (pollRef.current) clearInterval(pollRef.current);
      setWsConnected(false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session?.pairingCode, session?.status]);

  const handleCreate = () => {
    createMutation.mutate(
      { deviceType: deviceType as PairingSession['deviceType'] },
      {
        onSuccess: (s) => {
          setSession(s);
          setWsConnected(false);
        },
        onError: (err) => toast.error(err.message),
      }
    );
  };

  const handleSimulateConnect = () => {
    if (!session) return;
    connectMutation.mutate(
      {
        code: session.pairingCode,
        input: {
          deviceName: session.deviceName ?? `Paired ${deviceType}`,
          os: 'Auto-detected',
          region: 'US-East',
          connectionType: 'remote',
        },
      },
      {
        onSuccess: (s) => {
          setSession(s);
          if (pollRef.current) clearInterval(pollRef.current);
        },
        onError: (err) => toast.error(err.message),
      }
    );
  };

  const handleCancel = () => {
    if (!session) return;
    cancelMutation.mutate(session.pairingCode, {
      onSuccess: (s) => {
        setSession(s);
        if (pollRef.current) clearInterval(pollRef.current);
        setWsConnected(false);
      },
      onError: (err) => toast.error(err.message),
    });
  };

  const handleReset = () => {
    setSession(null);
    setQrDataUrl('');
    setWsConnected(false);
    if (pollRef.current) clearInterval(pollRef.current);
  };

  const status = session?.status ?? 'pending';
  const isPending = status === 'pending' && !countdown.expired;
  const isExpired = status === 'expired' || (status === 'pending' && countdown.expired);
  const isConnected = status === 'connected';

  return (
    <PageContainer>
      <Link
        to="/devices"
        className="mb-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to devices
      </Link>

      <motion.div initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
        <div className="mb-8 flex items-center gap-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <QrCode className="h-6 w-6" />
          </div>
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">QR Pairing</h1>
            <p className="text-sm text-muted-foreground">
              Scan the QR code with a device to instantly pair it with your workspace.
            </p>
          </div>
        </div>
      </motion.div>

      <div className="mx-auto max-w-3xl">
        <div className="grid gap-6 lg:grid-cols-2">
          {/* QR code panel */}
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Pairing code</CardTitle>
              <CardDescription>
                {session
                  ? 'Scan with your device camera or pairing app'
                  : 'Generate a code to start pairing'}
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col items-center gap-4">
              {!session && (
                <>
                  <div className="flex h-[280px] w-full items-center justify-center rounded-lg border-2 border-dashed bg-muted/30">
                    <QrCode className="h-16 w-16 text-muted-foreground/30" />
                  </div>
                  <div className="w-full space-y-3">
                    <div className="space-y-2">
                      <Label>Device type</Label>
                      <Select value={deviceType} onValueChange={setDeviceType}>
                        <SelectTrigger className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {deviceTypeOptions.map((t) => (
                            <SelectItem key={t.value} value={t.value}>
                              {t.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <Button
                      className="w-full"
                      onClick={handleCreate}
                      disabled={createMutation.isPending}
                    >
                      {createMutation.isPending ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <QrCode className="h-4 w-4" />
                      )}
                      Generate pairing code
                    </Button>
                  </div>
                </>
              )}

              {session && (
                <>
                  <motion.div
                    initial={{ opacity: 0, scale: 0.9 }}
                    animate={{ opacity: 1, scale: 1 }}
                    transition={{ type: 'spring', stiffness: 300, damping: 25 }}
                    className={`relative rounded-lg border-2 bg-white p-4 ${
                      isExpired ? 'opacity-40' : ''
                    } ${isConnected ? 'border-success' : 'border-border'}`}
                  >
                    {qrDataUrl ? (
                      <img src={qrDataUrl} alt="QR pairing code" className="h-[240px] w-[240px]" />
                    ) : (
                      <div className="flex h-[240px] w-[240px] items-center justify-center">
                        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
                      </div>
                    )}
                    {isConnected && (
                      <motion.div
                        initial={{ scale: 0 }}
                        animate={{ scale: 1 }}
                        className="absolute inset-0 flex items-center justify-center rounded-lg bg-success/10 backdrop-blur-sm"
                      >
                        <CheckCircle2 className="h-16 w-16 text-success" />
                      </motion.div>
                    )}
                    {isExpired && (
                      <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-background/80 backdrop-blur-sm">
                        <XCircle className="h-16 w-16 text-muted-foreground" />
                      </div>
                    )}
                  </motion.div>

                  {/* Pairing code text */}
                  <div className="rounded-lg border bg-muted/30 px-4 py-2.5">
                    <p className="font-mono text-sm font-semibold tracking-widest">
                      {session.pairingCode}
                    </p>
                  </div>

                  {/* Action buttons */}
                  <div className="flex w-full gap-2">
                    {isPending && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="flex-1"
                        onClick={handleCancel}
                        disabled={cancelMutation.isPending}
                      >
                        Cancel
                      </Button>
                    )}
                    {(isExpired || isConnected) && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="flex-1"
                        onClick={handleReset}
                      >
                        <RefreshCw className="h-4 w-4" />
                        {isConnected ? 'Pair another device' : 'Generate new code'}
                      </Button>
                    )}
                    {isConnected && session.connectedDeviceId && (
                      <Button
                        size="sm"
                        className="flex-1"
                        onClick={() => navigate(`/devices/${session.connectedDeviceId}`)}
                      >
                        View device
                      </Button>
                    )}
                  </div>
                </>
              )}
            </CardContent>
          </Card>

          {/* Status panel */}
          <div className="space-y-4">
            {/* Pairing status */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Pairing status</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <StatusRow
                  icon={isPending ? Loader2 : isConnected ? CheckCircle2 : isExpired ? XCircle : Clock}
                  iconClass={
                    isPending
                      ? 'animate-spin text-warning'
                      : isConnected
                      ? 'text-success'
                      : isExpired
                      ? 'text-destructive'
                      : 'text-muted-foreground'
                  }
                  label="Status"
                  value={
                    isConnected ? 'Connected' :
                    isExpired ? 'Expired' :
                    session?.status === 'cancelled' ? 'Cancelled' :
                    isPending ? 'Waiting for device…' : 'Not started'
                  }
                />

                {session && (
                  <>
                    <StatusRow
                      icon={Clock}
                      label="Time remaining"
                      value={countdown.display}
                      accent={countdown.expired ? 'text-destructive' : undefined}
                    />
                    <StatusRow
                      icon={Wifi}
                      label="WebSocket"
                      value={wsConnected ? 'Connected' : 'Connecting…'}
                      iconClass={wsConnected ? 'text-success' : 'text-warning'}
                    />
                    <StatusRow
                      icon={isPending ? Wifi : WifiOff}
                      label="Connection"
                      value={isConnected ? 'Established' : isPending ? 'Waiting' : 'No connection'}
                      iconClass={isConnected ? 'text-success' : ''}
                    />
                  </>
                )}
              </CardContent>
            </Card>

            {/* Instructions */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base">How to pair</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {[
                  'Open the VilSend app on the device you want to pair',
                  'Go to Settings and tap "Pair new workspace"',
                  'Point the camera at this QR code',
                  'Confirm the connection on both devices',
                ].map((step, i) => (
                  <div key={i} className="flex items-start gap-3">
                    <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold">
                      {i + 1}
                    </div>
                    <p className="pt-0.5 text-sm text-muted-foreground">{step}</p>
                  </div>
                ))}
              </CardContent>
            </Card>

            {/* Simulate connection button (for testing) */}
            {isPending && (
              <Card className="border-dashed">
                <CardContent className="flex items-center justify-between p-4">
                  <div>
                    <p className="text-sm font-medium">Simulate connection</p>
                    <p className="text-xs text-muted-foreground">For testing the pairing flow.</p>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleSimulateConnect}
                    disabled={connectMutation.isPending}
                  >
                    <Zap className="h-4 w-4" />
                    Connect
                  </Button>
                </CardContent>
              </Card>
            )}
          </div>
        </div>
      </div>
    </PageContainer>
  );
}

function StatusRow({
  icon: Icon,
  iconClass,
  label,
  value,
  accent,
}: {
  icon: typeof Clock;
  iconClass?: string;
  label: string;
  value: string;
  accent?: string;
}) {
  return (
    <div className="flex items-center justify-between">
      <div className="flex items-center gap-2.5">
        <Icon className={`h-4 w-4 ${iconClass ?? 'text-muted-foreground'}`} />
        <span className="text-sm text-muted-foreground">{label}</span>
      </div>
      <span className={`text-sm font-medium ${accent ?? ''}`}>{value}</span>
    </div>
  );
}
