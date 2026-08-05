import { api } from "@/lib/api";

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
  return data;
}

export async function updateTransferStatus(
  transferId: string,
  action: TransferAction,
): Promise<void> {
  await api.patch(`/transfers/${transferId}`, {
    action,
  });
}
