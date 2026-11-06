import { invoke } from "@tauri-apps/api/core";
import { api } from "@/lib/api";
import { getDeviceIdentifier } from "@/services/getDeviceInfo";

export type TransferConnectionType = "LAN" | "TUNNEL" | "lan" | "tunnel";

export type TransferStatus =
  | "PENDING"
  | "ACCEPTED"
  | "IN_PROGRESS"
  | "COMPLETED"
  | "FAILED"
  | "CANCELLED"
  | "UPLOADING"
  | "DOWNLOADING"
  | "PAUSED"
  | "CONNECTING"
  | "WAITING_SENDER"
  | "HANDSHAKING";

export type TransferAction = "ACCEPT" | "REJECT" | "CANCEL";

export type NetworkType = "LAN" | "REMOTE";

export interface TransferResponse {
  id: string;
  fileName: string;
  fileSize: number;
  status: TransferStatus;
  networkType: NetworkType;
  senderDeviceId: string;
  senderDeviceName: string;
  receiverDeviceId: string;
  receiverDeviceName: string;
  senderUserId: string;
  sentByMe: boolean;
  receiverUserId: string;
  createdAt: string;
  updatedAt: string;
  uploadedBytes: number;
  totalBytes: number;
  // Live fields; the REST payload does not carry them — the transfer-progress
  // listener fills them in while a transfer is running.
  percentage?: number;
  speed?: number; // bytes per second
  eta?: number | null; // seconds remaining
}

/**
 * The contract returned by the service that authorizes and routes a transfer.
 * Keep this boundary small: backend changes should be made here, not in the UI.
 */
export interface TransferMetadata {
  transferId: string;
  receiverId: string;
  connectionType: TransferConnectionType;
  endpoint: string;
  authToken: string;
  chunkSize?: number;
  concurrency?: number;
  maxRetries?: number;
}

export interface CreateTransferInput {
  receiverId: string;
  senderDeviceIdentifier: string;
  files: {
    name: string;
    size: number;
  }[];
}

export async function createTransfer(
  input: CreateTransferInput,
): Promise<TransferMetadata> {
  const { data } = await api.post<TransferMetadata>("/transfers", input);
  return data;
}

export async function getMyTransfers(): Promise<TransferResponse[]> {
  const { data } = await api.get<TransferResponse[]>("/transfers");

  const deviceIdentifier = await getDeviceIdentifier();

  return data.map((transfer) => ({
    ...transfer,

    // The current device is the sender only when its stored identifier
    // matches the device the backend recorded as the sender.
    sentByMe: transfer.senderDeviceId === deviceIdentifier,
  }));
}

export async function updateTransferStatus(
  transferId: string,
  action: TransferAction,
  transportManifest?: unknown,
): Promise<void> {
  await api.patch(`/transfers/${transferId}`, {
    action,
    // Always present on ACCEPT; `undefined` is dropped by JSON.stringify for
    // REJECT/CANCEL, which never probe. Never omit it here — an empty-but-valid
    // manifest is what tells the backend the receiver looked.
    transport_manifest: transportManifest,
  });
}

/**
 * Accepts a pending transfer. The transport probe must never block accepting,
 * so a failure (or an older build without the command) still sends a valid,
 * empty manifest — the backend records "present but empty" differently from
 * "missing".
 */
export async function acceptTransfer(transferId: string): Promise<void> {
  let transportManifest: unknown;

  try {
    transportManifest = await invoke("build_transport_manifest");
  } catch {
    transportManifest = {
      manifest_version: 1,
      capabilities: {
        p2p: false,
        lan: false,
        ble: false,
        preferred_order: [],
      },
    };
  }

  return updateTransferStatus(transferId, "ACCEPT", transportManifest);
}
