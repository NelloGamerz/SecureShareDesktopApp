// import { useEffect, useState } from "react";
// import { useClerk, useAuth as useClerkAuth } from "@clerk/clerk-react";

// import { getDeviceInfo } from "@/services/getDeviceInfo";

// import {
//   cloudflaredStatus,
//   startCloudflared,
//   stopCloudflared,
//   getConnectionStatus,
//   startTauriWebSocket,
//   stopTauriWebSocket,
//   onTransferRequest,
// } from "@/api/tauri";
// import { useNotificationStore } from "@/store/notification-store";
// import { useCurrentUserProfile } from "@/features/auth/auth-hooks";
// import { registerCurrentDevice } from "@/features/devices/devices-api";
// import { useTransferRequestStore } from "@/store/transfer-request-store";
// import axios from "axios";

// export function useDesktopServices() {
//   const { isLoaded, isSignedIn } = useClerkAuth();
//   const { signOut } = useClerk();
//   const { data: profile, isLoading } = useCurrentUserProfile();
//   const [deviceLimitReached, setDeviceLimitReached] = useState(false);
//   const [logoutCountdown, setLogoutCountdown] = useState(10);

//   useEffect(() => {
//     if (!isLoaded || isLoading) {
//       console.log("[DesktopServices] Waiting...", {
//         isLoaded,
//         isLoading,
//       });
//       return;
//     }

//     const syncServices = async () => {
//       console.log("[DesktopServices] Starting synchronization...");

//       try {
//         console.log("[DesktopServices] Checking current service status...");

//         const wsStatus = await getConnectionStatus();
//         const cloudflareRunning = await cloudflaredStatus();
//         const wsRunning = wsStatus === "Connected";

//         console.log("[DesktopServices] Current status:", {
//           wsRunning,
//           cloudflareRunning,
//           isSignedIn,
//           onboardingCompleted: profile?.onboardingCompleted,
//         });

//         const shouldRun = isSignedIn && profile?.onboardingCompleted === true;

//         console.log("[DesktopServices] shouldRun =", shouldRun);

//         if (shouldRun) {
//           if (profile.currentDeviceRegistered === false) {
//             console.log(
//               "[DesktopServices] Device not registered. Registering...",
//             );

//             try {
//               const result = await registerCurrentDevice();

//               console.log("[DesktopServices] Device registered:", result);
//             } catch (error) {
//               console.error(
//                 "[DesktopServices] Device registration failed:",
//                 error,
//               );

//               return;
//             }
//           }
//           if (!cloudflareRunning) {
//             console.log("[DesktopServices] Starting Cloudflared...");

//             try {
//               const result = await startCloudflared();
//               console.log("[DesktopServices] Cloudflared started:", result);
//             } catch (err) {
//               console.error(
//                 "[DesktopServices] Failed to start Cloudflared:",
//                 err,
//               );
//               throw err;
//             }
//           } else {
//             console.log("[DesktopServices] Cloudflared already running.");
//           }

//           if (!wsRunning) {
//             console.log("[DesktopServices] Getting device info...");

//             const deviceInfo = await getDeviceInfo();

//             console.log("[DesktopServices] Device info:", deviceInfo);

//             console.log("[DesktopServices] Starting WebSocket...");

//             try {
//               const result = await startTauriWebSocket(deviceInfo);

//               console.log("[DesktopServices] WebSocket started:", result);
//             } catch (err) {
//               console.error(
//                 "[DesktopServices] Failed to start WebSocket:",
//                 err,
//               );
//               throw err;
//             }
//           } else {
//             console.log("[DesktopServices] WebSocket already running.");
//           }

//           console.log(
//             "[DesktopServices] Synchronization completed successfully.",
//           );

//           return;
//         }

//         console.log("[DesktopServices] Services should NOT be running.");

//         if (wsRunning) {
//           console.log("[DesktopServices] Stopping WebSocket...");
//           await stopTauriWebSocket();
//           console.log("[DesktopServices] WebSocket stopped.");
//         }

//         if (cloudflareRunning) {
//           console.log("[DesktopServices] Stopping Cloudflared...");
//           await stopCloudflared();
//           console.log("[DesktopServices] Cloudflared stopped.");
//         }

//         console.log("[DesktopServices] Synchronization completed.");
//       } catch (error) {
//         console.error(
//           "[DesktopServices] Failed to synchronize desktop services:",
//           error,
//         );
//       }
//     };

//     void syncServices();
//   }, [
//     isLoaded,
//     isLoading,
//     isSignedIn,
//     profile?.onboardingCompleted,
//     profile?.currentDeviceRegistered,
//   ]);

//   useEffect(() => {
//     let unlisten: (() => void) | undefined;

//     const setupTransferListener = async () => {
//       unlisten = await onTransferRequest((request) => {
//         console.log("Transfer event received", request);

//         useTransferRequestStore.getState().addRequest(request);

//         useNotificationStore.getState().addNotification({
//           id: crypto.randomUUID(),
//           kind: "transfer",
//           title: "Incoming transfer request",
//           description: `${request.file_name} transfer request received`,
//           timestamp: "Just now",
//           read: false,
//         });
//       });

//       console.log(
//         "[Transfer Listener] Notifications:",
//         useNotificationStore.getState().notifications,
//       );
//     };

//     void setupTransferListener();

//     return () => {
//       unlisten?.();
//     };
//   }, []);
// }

import { useEffect, useState } from "react";
import { useClerk, useAuth as useClerkAuth } from "@clerk/clerk-react";
import axios from "axios";

import { getDeviceInfo } from "@/services/getDeviceInfo";

import {
  cloudflaredStatus,
  startCloudflared,
  stopCloudflared,
  getConnectionStatus,
  startTauriWebSocket,
  stopTauriWebSocket,
  onTransferRequest,
} from "@/api/tauri";

import { useNotificationStore } from "@/store/notification-store";
import { useCurrentUserProfile } from "@/features/auth/auth-hooks";
import { registerCurrentDevice } from "@/features/devices/devices-api";
import { useTransferRequestStore } from "@/store/transfer-request-store";

export function useDesktopServices() {
  const { isLoaded, isSignedIn } = useClerkAuth();
  const { signOut } = useClerk();

  const { data: profile, isLoading } = useCurrentUserProfile();

  const [deviceLimitReached, setDeviceLimitReached] = useState(false);
  const [logoutCountdown, setLogoutCountdown] = useState(10);

  // =========================================================
  // DEVICE LIMIT COUNTDOWN + AUTO LOGOUT
  // =========================================================

  useEffect(() => {
    if (!deviceLimitReached) {
      return;
    }

    console.log(
      "[DesktopServices] Device limit reached. Starting 10 second logout countdown.",
    );

    setLogoutCountdown(10);

    const interval = window.setInterval(() => {
      setLogoutCountdown((previous) => {
        if (previous <= 1) {
          window.clearInterval(interval);
          return 0;
        }

        return previous - 1;
      });
    }, 1000);

    const timeout = window.setTimeout(async () => {
      console.log(
        "[DesktopServices] Countdown finished. Logging out from Clerk...",
      );

      try {
        await signOut();

        console.log("[DesktopServices] Clerk logout successful.");
      } catch (error) {
        console.error("[DesktopServices] Clerk logout failed:", error);
      }
    }, 10000);

    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [deviceLimitReached, signOut]);

  // =========================================================
  // DESKTOP SERVICES
  // =========================================================

  useEffect(() => {
    if (!isLoaded || isLoading) {
      console.log("[DesktopServices] Waiting...", {
        isLoaded,
        isLoading,
      });

      return;
    }

    // Don't start/restart services after device limit is reached.
    if (deviceLimitReached) {
      console.log(
        "[DesktopServices] Device limit reached. Skipping service synchronization.",
      );

      return;
    }

    const syncServices = async () => {
      console.log("[DesktopServices] Starting synchronization...");

      try {
        console.log("[DesktopServices] Checking current service status...");

        const wsStatus = await getConnectionStatus();
        const cloudflareRunning = await cloudflaredStatus();
        const wsRunning = wsStatus === "Connected";

        console.log("[DesktopServices] Current status:", {
          wsRunning,
          cloudflareRunning,
          isSignedIn,
          onboardingCompleted: profile?.onboardingCompleted,
        });

        const shouldRun = isSignedIn && profile?.onboardingCompleted === true;

        console.log("[DesktopServices] shouldRun =", shouldRun);

        // =====================================================
        // START SERVICES
        // =====================================================

        if (shouldRun) {
          // ---------------------------------------------------
          // DEVICE REGISTRATION
          // ---------------------------------------------------

          if (profile.currentDeviceRegistered === false) {
            console.log(
              "[DesktopServices] Device not registered. Registering...",
            );

            try {
              const result = await registerCurrentDevice();

              console.log("[DesktopServices] Device registered:", result);
            } catch (error: unknown) {
              console.error(
                "[DesktopServices] Device registration failed:",
                error,
              );

              // -----------------------------------------------
              // DEVICE LIMIT REACHED
              // -----------------------------------------------

              if (axios.isAxiosError(error) && error.response?.status === 402) {
                console.log("[DesktopServices] Device limit reached.");

                setDeviceLimitReached(true);
                setLogoutCountdown(10);

                // IMPORTANT:
                // Do not start Cloudflared.
                // Do not start WebSocket.
                return;
              }

              // Other registration errors.
              return;
            }
          }

          // ---------------------------------------------------
          // CLOUDFLARED
          // ---------------------------------------------------

          if (!cloudflareRunning) {
            console.log("[DesktopServices] Starting Cloudflared...");

            try {
              const result = await startCloudflared();

              console.log("[DesktopServices] Cloudflared started:", result);
            } catch (error) {
              console.error(
                "[DesktopServices] Failed to start Cloudflared:",
                error,
              );

              throw error;
            }
          } else {
            console.log("[DesktopServices] Cloudflared already running.");
          }

          // ---------------------------------------------------
          // WEBSOCKET
          // ---------------------------------------------------

          if (!wsRunning) {
            console.log("[DesktopServices] Getting device info...");

            const deviceInfo = await getDeviceInfo();

            console.log("[DesktopServices] Device info:", deviceInfo);

            console.log("[DesktopServices] Starting WebSocket...");

            try {
              const result = await startTauriWebSocket(deviceInfo);

              console.log("[DesktopServices] WebSocket started:", result);
            } catch (error) {
              console.error(
                "[DesktopServices] Failed to start WebSocket:",
                error,
              );

              throw error;
            }
          } else {
            console.log("[DesktopServices] WebSocket already running.");
          }

          console.log(
            "[DesktopServices] Synchronization completed successfully.",
          );

          return;
        }

        // =====================================================
        // STOP SERVICES
        // =====================================================

        console.log("[DesktopServices] Services should NOT be running.");

        if (wsRunning) {
          console.log("[DesktopServices] Stopping WebSocket...");

          await stopTauriWebSocket();

          console.log("[DesktopServices] WebSocket stopped.");
        }

        if (cloudflareRunning) {
          console.log("[DesktopServices] Stopping Cloudflared...");

          await stopCloudflared();

          console.log("[DesktopServices] Cloudflared stopped.");
        }

        console.log("[DesktopServices] Synchronization completed.");
      } catch (error) {
        console.error(
          "[DesktopServices] Failed to synchronize desktop services:",
          error,
        );
      }
    };

    void syncServices();
  }, [
    isLoaded,
    isLoading,
    isSignedIn,
    profile?.onboardingCompleted,
    profile?.currentDeviceRegistered,
    deviceLimitReached,
  ]);

  // =========================================================
  // TRANSFER REQUEST LISTENER
  // =========================================================

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupTransferListener = async () => {
      unlisten = await onTransferRequest((request) => {
        console.log("Transfer event received", request);

        useTransferRequestStore.getState().addRequest(request);

        useNotificationStore.getState().addNotification({
          id: crypto.randomUUID(),
          kind: "transfer",
          title: "Incoming transfer request",
          description: `${request.file_name} transfer request received`,
          timestamp: "Just now",
          read: false,
        });
      });

      console.log(
        "[Transfer Listener] Notifications:",
        useNotificationStore.getState().notifications,
      );
    };

    void setupTransferListener();

    return () => {
      unlisten?.();
    };
  }, []);

  // =========================================================
  // RETURN STATE FOR UI
  // =========================================================

  return {
    deviceLimitReached,
    logoutCountdown,
  };
}
