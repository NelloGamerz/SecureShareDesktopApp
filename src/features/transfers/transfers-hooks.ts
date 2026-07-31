import { useEffect } from "react";
import { UnlistenFn } from "@tauri-apps/api/event";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  createTransfer,
  getMyTransfers,
  TransferAction,
  updateTransferStatus,
  type CreateTransferInput,
  type TransferMetadata,
} from "./transfers-api";

import { pauseTransfer, resumeTransfer, cancelTransfer, TransferEvent, onTransferEvent } from "@/api/tauri";

// type TransferProgressPayload = {
//   transferId: string;
//   uploadedBytes: number;
//   totalBytes: number;
//   percentage: number;
//   speed: number;
//   eta: number | null;
//   status?: string;
// };

export function useCreateTransfer() {
  const queryClient = useQueryClient();

  return useMutation<TransferMetadata, Error, CreateTransferInput>({
    mutationFn: createTransfer,

    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["transfers"],
      });
    },
  });
}

export function useTransfers() {
  return useQuery({
    queryKey: ["transfers"],
    queryFn: getMyTransfers,
  });
}

export function useTransferAction() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      transferId,
      action,
    }: {
      transferId: string;
      action: TransferAction;
    }) => updateTransferStatus(transferId, action),

    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["transfers"],
      });
    },
  });
}

export function useTransferControls() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      transferId,
      action,
    }: {
      transferId: string;
      action: "PAUSE" | "RESUME" | "CANCEL";
    }) => {
      switch (action) {
        case "PAUSE":
          return pauseTransfer(transferId);

        case "RESUME":
          return resumeTransfer(transferId);

        case "CANCEL":
          return cancelTransfer(transferId);
      }
    },

    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["transfers"],
      });
    },
  });
}

// export function useTransferProgressListener() {
//   const queryClient = useQueryClient();

//   useEffect(() => {
//     let unlisten: (() => void) | undefined;

//     async function setup() {
//       unlisten = await listen<TransferProgressPayload>(
//         "transfer-progress",

//         ({ payload }) => {
//           console.log("TRANSFER PROGRESS EVENT", payload);

//           queryClient.setQueryData(
//             ["transfers"],

//             (old: any[] | undefined) => {
//               if (!old) return old;

//               return old.map((transfer) => {
//                 if (transfer.id !== payload.transferId) {
//                   return transfer;
//                 }

//                 return {
//                   ...transfer,

//                   uploadedBytes: payload.uploadedBytes,

//                   totalBytes: payload.totalBytes,

//                   percentage: payload.percentage,

//                   speed: payload.speed,

//                   eta: payload.eta,

//                   // only update if backend sends it
//                   ...(payload.status && {
//                     status: payload.status,
//                   }),
//                 };
//               });
//             },
//           );
//         },
//       );
//     }

//     setup();

//     return () => {
//       unlisten?.();
//     };
//   }, [queryClient]);
// }

export function useTransferProgressListener() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const events: TransferEvent[] = [
      "transfer-progress",
      "transfer-completed",
      "transfer-failed",
      "transfer-paused",
      "transfer-resumed",
      "transfer-cancelled",
    ];

    const unlisteners: UnlistenFn[] = [];

    async function setup() {
      for (const event of events) {
        const unlisten = await onTransferEvent(event, (payload) => {
          console.log(event, payload);

          queryClient.setQueryData(["transfers"], (old: any[] | undefined) => {
            if (!old) return old;

            return old.map((transfer) => {
              if (transfer.id !== payload.transfer_id) {
                return transfer;
              }

              return {
                ...transfer,

                uploadedBytes: payload.uploaded_bytes,
                totalBytes: payload.total_bytes,

                percentage: payload.percentage,

                speed: payload.speed,

                eta: payload.eta,

                status: payload.status ?? transfer.status,
              };
            });
          });
        });

        unlisteners.push(unlisten);
      }
    }

    setup();

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [queryClient]);
}
