import { api } from "@/lib/api";
import { getDeviceIdentifier } from "@/services/getDeviceInfo";

/** Tunnel settings as provisioned for this device by the central API. */
export interface TunnelInfo {
  hostname: string;
  tunnelToken?: string | null;
}

/**
 * GET /tunnel/info — the tunnel hostname for the authenticated device.
 *
 * Read-only, unlike `POST /onboarding`, which also creates the workspace and
 * tunnel. Used to restore the hostname when the keychain copy is missing.
 */
export async function getTunnelInfo(): Promise<TunnelInfo> {
  const deviceIdentifier = await getDeviceIdentifier();

  const { data } = await api.get<TunnelInfo>("/tunnel/info", {
    headers: { "X-Device-Id": deviceIdentifier },
  });

  if (!data?.hostname) {
    throw new Error("Tunnel info response did not include a hostname.");
  }

  return data;
}
