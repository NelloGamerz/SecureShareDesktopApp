import { get_tunnel_hostname, save_tunnel_hostname } from "@/api/tauri";
import { getTunnelInfo } from "./tunnel-api";

/**
 * Ensures the tunnel hostname is present in the OS keychain.
 *
 * Onboarding writes it, but a session restored elsewhere — or a keychain that
 * was cleared — leaves the Rust side without a hostname, and Cloudflared then
 * has nothing to advertise. When the keychain read fails for any reason, the
 * hostname is re-read from the central API and written back.
 *
 * Returns the hostname, or `null` when neither source could supply one.
 */
export async function ensureTunnelHostname(): Promise<string | null> {
  try {
    const stored = await get_tunnel_hostname();

    if (stored) {
      return stored;
    }
  } catch (error) {
    console.warn(
      "[Tunnel] No hostname in the keychain, fetching it from the API:",
      error,
    );
  }

  try {
    const { hostname } = await getTunnelInfo();

    await save_tunnel_hostname(hostname);

    console.log("[Tunnel] Hostname fetched from the API and stored.");

    return hostname;
  } catch (error) {
    console.error("[Tunnel] Failed to fetch the tunnel hostname:", error);

    return null;
  }
}
