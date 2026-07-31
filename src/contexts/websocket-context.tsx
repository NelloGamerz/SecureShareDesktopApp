import { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { ConnectionStatus, ServerEvent, WebSocketMessage } from '@/types/auth';

interface WebSocketContextValue {
  status: ConnectionStatus;
  connected: boolean;
  messages: WebSocketMessage[];
  send: (payload: string) => Promise<void>;
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null);

export function WebSocketProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');
  const [messages, setMessages] = useState<WebSocketMessage[]>([]);

  useEffect(() => {
    const unsubscribe = listen<ServerEvent>('server-event', (event) => {
      if (event.payload.type === 'connection-status') {
        setStatus((event.payload.payload as ConnectionStatus) ?? 'disconnected');
      }
    });

    const unsubscribeMessages = listen<WebSocketMessage>('websocket-message', (event) => {
      setMessages((current) => [...current, event.payload]);
    });

    return () => {
      void unsubscribe.then((fn) => fn());
      void unsubscribeMessages.then((fn) => fn());
    };
  }, []);

  const send = async (payload: string) => {
    const { sendTauriMessage } = await import('@/api/tauri');
    await sendTauriMessage(payload);
  };

  const value = useMemo(() => ({ status, connected: status === 'connected', messages, send }), [messages, status]);

  return <WebSocketContext.Provider value={value}>{children}</WebSocketContext.Provider>;
}

export function useWebSocket() {
  const context = useContext(WebSocketContext);
  if (!context) {
    throw new Error('useWebSocket must be used within WebSocketProvider');
  }
  return context;
}
