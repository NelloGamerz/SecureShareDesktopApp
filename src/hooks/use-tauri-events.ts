import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

export function useTauriEvents<T>(eventName: string, handler: (payload: T) => void) {
  useEffect(() => {
    let mounted = true;
    const subscribe = async () => {
      const unlisten = await listen<T>(eventName, (event) => {
        if (mounted) {
          handler(event.payload);
        }
      });
      return unlisten;
    };

    void subscribe();
    return () => {
      mounted = false;
    };
  }, [eventName, handler]);
}
