import { create } from "zustand";
import type { TransferRequestPayload } from "@/api/tauri";

interface TransferRequestState {
  requests: TransferRequestPayload[];
  addRequest: (request: TransferRequestPayload) => void;
  removeRequest: (transferId: string) => void;
}

export const useTransferRequestStore = create<TransferRequestState>((set) => ({
  requests: [],

  // addRequest: (request) =>
  //   set((state) => ({
  //     requests: [
  //       ...state.requests,
  //       request,
  //     ],
  //   })),

  addRequest: (request) =>
    set((state) => {
      const alreadyExists = state.requests.some(
        (existingRequest) =>
          existingRequest.transfer_id === request.transfer_id,
      );

      if (alreadyExists) {
        return state;
      }

      return {
        requests: [...state.requests, request],
      };
    }),

  removeRequest: (transferId) =>
    set((state) => ({
      requests: state.requests.filter(
        (request) => request.transfer_id !== transferId,
      ),
    })),
}));
