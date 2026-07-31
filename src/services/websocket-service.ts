import { sendTauriMessage } from '@/api/tauri';

export async function sendToTauriWebSocket(payload: string) {
  await sendTauriMessage(payload);
}
