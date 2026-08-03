import { useEffect } from "react";
import { useAuth as useClerkAuth } from "@clerk/clerk-react";

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

export function useDesktopServices() {
  const { isLoaded, isSignedIn } = useClerkAuth();
  const { data: profile, isLoading } = useCurrentUserProfile();

  useEffect(() => {
    if (!isLoaded || isLoading) {
      console.log("[DesktopServices] Waiting...", {
        isLoaded,
        isLoading,
      });
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

        if (shouldRun) {
          if (profile.currentDeviceRegistered === false) {
            console.log(
              "[DesktopServices] Device not registered. Registering...",
            );

            try {
              const result = await registerCurrentDevice();

              console.log("[DesktopServices] Device registered:", result);
            } catch (error) {
              console.error(
                "[DesktopServices] Device registration failed:",
                error,
              );

              return;
            }
          }
          if (!cloudflareRunning) {
            console.log("[DesktopServices] Starting Cloudflared...");

            try {
              const result = await startCloudflared();
              console.log("[DesktopServices] Cloudflared started:", result);
            } catch (err) {
              console.error(
                "[DesktopServices] Failed to start Cloudflared:",
                err,
              );
              throw err;
            }
          } else {
            console.log("[DesktopServices] Cloudflared already running.");
          }

          if (!wsRunning) {
            console.log("[DesktopServices] Getting device info...");

            const deviceInfo = await getDeviceInfo();

            console.log("[DesktopServices] Device info:", deviceInfo);

            console.log("[DesktopServices] Starting WebSocket...");

            try {
              const result = await startTauriWebSocket(deviceInfo);

              console.log("[DesktopServices] WebSocket started:", result);
            } catch (err) {
              console.error(
                "[DesktopServices] Failed to start WebSocket:",
                err,
              );
              throw err;
            }
          } else {
            console.log("[DesktopServices] WebSocket already running.");
          }

          console.log(
            "[DesktopServices] Synchronization completed successfully.",
          );

          return;
        }

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
  ]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const setupTransferListener = async () => {
      unlisten = await onTransferRequest((request) => {
        console.log("Transfer event received", request);
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
}
