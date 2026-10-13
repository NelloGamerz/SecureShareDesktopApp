import {
  get_tunnel_hostname,
  get_tunnel_token,
  save_tunnel_hostname,
  save_tunnel_token,
} from "@/api/tauri";
import { getTunnelInfo } from "./tunnel-api";

/**
 * Ensures the tunnel hostname and token are present in the OS keychain.
 *
 * Onboarding writes both, but a session restored elsewhere — or a keychain that
 * was cleared — leaves the Rust side without them, and Cloudflared then has
 * nothing to advertise and no credentials to open the tunnel with. It reads
 * both from the keychain before starting, so either one missing fails the
 * spawn. When either keychain read fails, both are re-read from the central API
 * and written back.
 *
 * Returns the hostname, or `null` when neither source could supply one.
 */
export async function ensureTunnelHostname(): Promise<string | null> {
  try {
    const stored = await get_tunnel_hostname();
    const storedToken = await get_tunnel_token();

    if (stored && storedToken) {
      return stored;
    }
  } catch (error) {
    console.warn(
      "[Tunnel] Tunnel hostname or token missing from the keychain, fetching both from the API:",
      error,
    );
  }

  try {
    const { hostname, tunnelToken } = await getTunnelInfo();

    if (!tunnelToken) {
      throw new Error("Tunnel info response did not include a tunnel token.");
    }

    await save_tunnel_hostname(hostname);
    await save_tunnel_token(tunnelToken);

    console.log("[Tunnel] Hostname and token fetched from the API and stored.");

    return hostname;
  } catch (error) {
    console.error("[Tunnel] Failed to fetch the tunnel credentials:", error);

    return null;
  }
}
