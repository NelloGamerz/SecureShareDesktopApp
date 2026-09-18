import { useEffect, useState } from "react";
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
import { ensureTunnelHostname } from "@/features/tunnel/tunnel-hostname";
import { useCurrentUserProfile } from "@/features/auth/auth-hooks";
import { useAuth } from "@/contexts/auth-context";
import { registerCurrentDevice } from "@/features/devices/devices-api";
import { useTransferRequestStore } from "@/store/transfer-request-store";

export function useDesktopServices() {
  const { isLoaded, isAuthenticated: isSignedIn, logout } = useAuth();

  const { data: profile, isLoading } = useCurrentUserProfile({
    enabled: isSignedIn,
  });

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
        "[DesktopServices] Countdown finished. Ending the desktop session...",
      );

      try {
        // Stops the WebSocket and Cloudflared, clears the Rust session, and
        // moves the app back to the sign-in screen.
        await logout();

        console.log("[DesktopServices] Logout successful.");
      } catch (error) {
        console.error("[DesktopServices] Logout failed:", error);
      }
    }, 10000);

    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [deviceLimitReached, logout]);

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
          // TUNNEL HOSTNAME
          // ---------------------------------------------------

          /*
           * Cloudflared reads its hostname from the keychain, so repair a
           * missing hostname before starting it rather than letting the spawn
           * fail on a lookup error.
           */
          const hostname = await ensureTunnelHostname();

          if (!hostname) {
            console.error(
              "[DesktopServices] Tunnel hostname unavailable. Skipping Cloudflared start.",
            );
          }

          // ---------------------------------------------------
          // CLOUDFLARED
          // ---------------------------------------------------

          if (!cloudflareRunning && hostname) {
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
