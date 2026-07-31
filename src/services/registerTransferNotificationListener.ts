// import { onTransferRequest } from "@/api/tauri";
// import { useNotificationStore } from "@/store/notification-store";
import { onTransferRequest } from "@/api/tauri";
import { useNotificationStore } from "@/store/notification-store";
import { useTransferRequestStore } from "@/store/transfer-request-store";


export async function registerTransferNotificationListener() {
  return onTransferRequest((request) => {
    // store request for Transfers page
    useTransferRequestStore.getState().addRequest(request);

    // create notification
    useNotificationStore.getState().addNotification({
      id: crypto.randomUUID(),
      kind: "transfer",
      title: "Incoming transfer request",
      description: `${request.file_name} received from device`,
      timestamp: "Just now",
      read: false,
    });
  });
}
