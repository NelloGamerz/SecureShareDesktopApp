import { useEffect, useMemo } from "react";
import { UnlistenFn } from "@tauri-apps/api/event";
import type { TransferProgress } from "@/api/tauri";
import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
} from "@tanstack/react-query";

import {
  acceptTransfer,
  createTransfer,
  getMyTransfers,
  TransferAction,
  updateTransferStatus,
  type CreateTransferInput,
  type TransferMetadata,
  type TransferResponse,
  type TransferStatus,
  type TransfersPage,
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
  const queryClient = useQueryClient();

  const query = useInfiniteQuery({
    queryKey: ["transfers"],
    queryFn: ({ pageParam }) => getMyTransfers(pageParam),
    initialPageParam: undefined as string | undefined,
    // No next cursor (or hasMore === false) means stop paging.
    getNextPageParam: (lastPage) =>
      lastPage.hasMore && lastPage.nextCursor ? lastPage.nextCursor : undefined,
    // A bad cursor (400) must not be retried; other failures keep the default retries.
    retry: (failureCount, error) =>
      (error as { status?: number }).status !== 400 && failureCount < 3,
  });

  const errorStatus = (query.error as { status?: number } | null)?.status;

  useEffect(() => {
    // An expired/invalid cursor is a 400; retrying it can never succeed, so
    // restart from the first page instead.
    if (errorStatus === 400) {
      void queryClient.resetQueries({ queryKey: ["transfers"] });
    }
  }, [errorStatus, queryClient]);

  const transfers = useMemo(() => {
    // Pages can overlap when rows change between requests; keep the first id.
    const byId = new Map<string, TransferResponse>();

    for (const page of query.data?.pages ?? []) {
      for (const transfer of page.items) {
        if (!byId.has(transfer.id)) byId.set(transfer.id, transfer);
      }
    }

    return [...byId.values()];
  }, [query.data]);

  return {
    transfers,
    isLoading: query.isLoading,
    isError: query.isError,
    hasMore: query.hasNextPage,
    isFetchingMore: query.isFetchingNextPage,
    fetchMore: query.fetchNextPage,
  };
}

export function useTransferAction() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      transferId,
      action,
    }: {
      transferId: string;
      action: TransferAction;
    }) => {
      if (action === "ACCEPT") {
        return acceptTransfer(transferId);
      }

      return updateTransferStatus(transferId, action);
    },

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

    // Progress arrives once per chunk; keep only the latest sample per transfer
    // and write the cache at most twice a second instead of per chunk.
    const pending = new Map<string, TransferProgress>();

    const flush = () => {
      if (pending.size === 0) return;

      const updates = new Map(pending);
      pending.clear();

      queryClient.setQueryData<InfiniteData<TransfersPage>>(
        ["transfers"],
        (old) =>
          old && {
            ...old,
            pages: old.pages.map((page) => ({
              ...page,
              items: page.items.map((transfer) => {
                const payload = updates.get(transfer.id);
                if (!payload) return transfer;

                return {
                  ...transfer,

                  uploadedBytes: payload.uploaded_bytes,
                  totalBytes: payload.total_bytes,

                  percentage: payload.percentage,

                  speed: payload.speed,

                  eta: payload.eta,

                  status:
                    (payload.status as TransferStatus | undefined) ??
                    transfer.status,
                };
              }),
            })),
          },
      );
    };

    async function setup() {
      for (const event of events) {
        const unlisten = await onTransferEvent(event, (payload) => {
          pending.set(payload.transfer_id, payload);

          // Terminal/state changes are rare and must land immediately.
          if (event !== "transfer-progress") flush();
        });

        unlisteners.push(unlisten);
      }
    }

    void setup();
    const timer = window.setInterval(flush, 500);

    return () => {
      window.clearInterval(timer);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [queryClient]);
}
