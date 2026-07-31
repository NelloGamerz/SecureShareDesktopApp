import { invoke } from "@tauri-apps/api/core";
import { DeviceInfo } from "@/features/onboarding/onboarding-types";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

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

export async function loginWithTauri(token: string, deviceInfo: DeviceInfo) {
  return invoke<void>("login", { token, deviceInfo });
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

export async function getTauriConnectionStatus() {
  return invoke<string>("get_connection_status");
}

export async function save_tunnel_token(token: string) {
  return invoke<void>("save_tunnel_token", { token });
}

export async function save_tunnel_hostname(hostname: string) {
  return invoke<void>("save_tunnel_hostname", { hostname });
}

export async function startCloudflared() {
  return invoke<void>("start_cloudflared_cmd");
}

export async function stopCloudflared() {
  return invoke<void>("stop_cloudflared_cmd");
}

export function getConnectionStatus(): Promise<string> {
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
  return listen<TransferRequestPayload>("transfer-request", ({ payload }) => {
    callback(payload);
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
  return invoke<boolean>("local_transfer_exists", {
    transferId,
  });
}

export async function updateAuthToken(token: string) {
  return invoke<void>("update_auth_token", { token });
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
