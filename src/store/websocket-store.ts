import { create } from 'zustand';
import type { ConnectionStatus, WebSocketMessage } from '@/types/auth';

interface WebSocketStoreState {
  status: ConnectionStatus;
  messages: WebSocketMessage[];
  setStatus: (status: ConnectionStatus) => void;
  addMessage: (message: WebSocketMessage) => void;
}

export const useWebSocketStore = create<WebSocketStoreState>((set) => ({
  status: 'disconnected',
  messages: [],
  setStatus: (status) => set({ status }),
  addMessage: (message) => set((state) => ({ messages: [...state.messages, message] })),
}));
