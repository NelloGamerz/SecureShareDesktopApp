import { invoke } from "@tauri-apps/api/core";
import { DeviceInfo } from "@/features/onboarding/onboarding-types";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ConnectionStatus } from "@/types/auth";

export interface TransferProgress {
  transfer_id: string;
  uploaded_bytes: number;
  total_bytes: number;
  uploaded_chunks: number;
  total_chunks: number;
  retry_count: number;
  status: string;
  percentage: number;
  speed: number;
  eta: number | null;
}

export interface LocalTransferFile {
  transfer_id: string;
  file_path: string;
  file_name: string;
  file_size: number;
}

export interface TransferRequestPayload {
  id: string;
  transfer_id: string;
  sender_device_id: string;
  receiver_device_id: string;
  file_name: string;
  file_size: number;
}

export type TransferEvent =
  | "transfer-progress"
  | "transfer-completed"
  | "transfer-failed"
  | "transfer-paused"
  | "transfer-resumed"
  | "transfer-cancelled"
  | "transfer-request";

/** Non-sensitive view of the desktop session held by Rust. */
export interface TauriAuthStatus {
  isAuthenticated: boolean;
  userId?: string | null;
  isAuthenticating: boolean;
}

export interface DesktopAuthConfig {
  clientId: string;
  issuer: string;
  redirectUri: string;
  scopes: string;
  signupPrompt?: string;
}

/**
 * Starts the system-browser Authorization Code + PKCE sign-in.
 *
 * Rust mints the PKCE verifier, opens the browser, and stores the resulting
 * token. Nothing sensitive crosses this boundary, so no token is returned.
 */
export async function startDesktopAuth(
  config: DesktopAuthConfig,
  mode: "sign_in" | "sign_up",
) {
  return invoke<void>("start_desktop_auth", { config, mode });
}

/** Abandons an in-flight sign-in. Safe to call when none is pending. */
export async function cancelDesktopAuth() {
  return invoke<void>("cancel_desktop_auth");
}

export async function getAuthStatus() {
  return invoke<TauriAuthStatus>("get_auth_status");
}

/**
 * Returns a token for the `Authorization` header, refreshing it when close to
 * expiry. Resolves to `null` when there is no session.
 */
export async function getAuthToken() {
  return invoke<string | null>("get_auth_token");
}

export async function logoutFromTauri() {
  return invoke<void>("logout");
}

export async function startTauriWebSocket(deviceInfo: DeviceInfo) {
  return invoke<void>("start_websocket", {
    deviceInfo,
  });
}

export async function stopTauriWebSocket() {
  return invoke<void>("stop_websocket");
}

export async function sendTauriMessage(payload: string) {
  return invoke<void>("send_message", { payload });
}

export async function save_tunnel_token(token: string) {
  return invoke<void>("save_tunnel_token", { token });
}

export async function save_tunnel_hostname(hostname: string) {
  return invoke<void>("save_tunnel_hostname", { hostname });
}

export async function get_tunnel_hostname() {
  return invoke<string>("get_tunnel_hostname");
}

export async function delete_tunnel_hostname() {
  return invoke<void>("delete_tunnel_hostname");
}

export async function startCloudflared() {
  return invoke<void>("start_cloudflared_cmd");
}

export async function stopCloudflared() {
  return invoke<void>("stop_cloudflared_cmd");
}

export function getConnectionStatus(): Promise<ConnectionStatus> {
  return invoke("get_connection_status");
}

export function cloudflaredStatus(): Promise<boolean> {
  return invoke("cloudflared_status");
}

export async function createDeviceIdentity(): Promise<string> {
  return invoke<string>("create_device_identity");
}

export function onTransferEvent(
  event: TransferEvent,
  callback: (progress: TransferProgress) => void,
): Promise<UnlistenFn> {
  return listen<TransferProgress>(event, ({ payload }) => callback(payload));
}

export function onTransferRequest(
  callback: (request: TransferRequestPayload) => void,
): Promise<UnlistenFn> {
  return listen<{
    transferId?: string;
    transfer_id?: string;
    senderDeviceId?: string;
    sender_device_id?: string;
    receiverDeviceId?: string;
    receiver_device_id?: string;
    fileName?: string;
    file_name?: string;
    fileSize?: number;
    file_size?: number;
    id?: string;
  }>("transfer-request", ({ payload }) => {
    const normalizedRequest: TransferRequestPayload = {
      id: payload.id ?? payload.transferId ?? payload.transfer_id ?? "",
      transfer_id: payload.transferId ?? payload.transfer_id ?? payload.id ?? "",
      sender_device_id:
        payload.senderDeviceId ?? payload.sender_device_id ?? "",
      receiver_device_id:
        payload.receiverDeviceId ?? payload.receiver_device_id ?? "",
      file_name: payload.fileName ?? payload.file_name ?? "",
      file_size: payload.fileSize ?? payload.file_size ?? 0,
    };

    callback(normalizedRequest);
  });
}

export async function saveLocalTransferFile(file: LocalTransferFile) {
  return invoke<void>("save_local_transfer_file", {
    file,
  });
}

export async function getLocalTransferFiles(
  transferId: string,
): Promise<LocalTransferFile[]> {
  return invoke<LocalTransferFile[]>("get_local_transfer_files", {
    transferId,
  });
}

export async function deleteLocalTransferFiles(transferId: string) {
  return invoke<void>("delete_local_transfer_files", {
    transferId,
  });
}

export async function deleteLocalTransferFile(
  transferId: string,
  filePath: string,
) {
  return invoke<void>("delete_local_transfer_file", {
    transferId,
    filePath,
  });
}

export async function localTransferExists(
  transferId: string,
): Promise<boolean> {
  return invoke<boolean>("check_local_transfer_exists", {
    transferId,
  });
}

export async function pauseTransfer(id: string) {
  return invoke("pause_transfer", { id });
}

export async function resumeTransfer(id: string) {
  return invoke("resume_transfer", { id });
}

export async function cancelTransfer(id: string) {
  return invoke("cancel_transfer", { id });
}

export async function detectDeviceType(): Promise<string> {
  return invoke<string>("detect_device_type");
}

export async function getDefaultDownloadLocation(): Promise<string> {
  return invoke<string>("get_default_download_location");
}

export async function setDefaultDownloadLocation(path: string) {
  return invoke<void>("set_default_download_location", { path });
}
