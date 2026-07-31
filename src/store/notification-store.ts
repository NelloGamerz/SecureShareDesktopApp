// import { create } from 'zustand';

// export type NotificationKind = 'transfer' | 'device' | 'member' | 'billing' | 'system';

// export interface AppNotification {
//   id: string;
//   kind: NotificationKind;
//   title: string;
//   description: string;
//   timestamp: string;
//   read: boolean;
// }

// interface NotificationState {
//   notifications: AppNotification[];
//   unreadCount: () => number;
//   markAllRead: () => void;
//   markRead: (id: string) => void;
//   clearAll: () => void;
// }

// const seed: AppNotification[] = [
//   {
//     id: 'n1',
//     kind: 'transfer',
//     title: 'Transfer completed',
//     description: 'Q4-financials.zip sent to 3 recipients',
//     timestamp: '2m ago',
//     read: false,
//   },
//   {
//     id: 'n2',
//     kind: 'device',
//     title: 'New device connected',
//     description: 'MacBook Pro — San Francisco, CA',
//     timestamp: '1h ago',
//     read: false,
//   },
//   {
//     id: 'n3',
//     kind: 'member',
//     title: 'Member joined',
//     description: 'alex.rivera@helix.io accepted your invite',
//     timestamp: '3h ago',
//     read: false,
//   },
//   {
//     id: 'n4',
//     kind: 'billing',
//     title: 'Invoice available',
//     description: 'Your July invoice for $240.00 is ready',
//     timestamp: '1d ago',
//     read: true,
//   },
//   {
//     id: 'n5',
//     kind: 'system',
//     title: 'Security alert',
//     description: 'New sign-in from Chrome on macOS',
//     timestamp: '2d ago',
//     read: true,
//   },
// ];

// export const useNotificationStore = create<NotificationState>((set, get) => ({
//   notifications: seed,
//   unreadCount: () => get().notifications.filter((n) => !n.read).length,
//   markAllRead: () =>
//     set((s) => ({
//       notifications: s.notifications.map((n) => ({ ...n, read: true })),
//     })),
//   markRead: (id) =>
//     set((s) => ({
//       notifications: s.notifications.map((n) =>
//         n.id === id ? { ...n, read: true } : n
//       ),
//     })),
//   clearAll: () => set({ notifications: [] }),
// }));

import { create } from "zustand";

export type NotificationKind =
  | "transfer"
  | "device"
  | "member"
  | "billing"
  | "system";

export interface AppNotification {
  id: string;
  kind: NotificationKind;
  title: string;
  description: string;
  timestamp: string;
  read: boolean;
}

interface NotificationState {
  notifications: AppNotification[];
  unreadCount: () => number;
  addNotification: (notification: AppNotification) => void;
  markAllRead: () => void;
  markRead: (id: string) => void;
  clearAll: () => void;
}

export const useNotificationStore = create<NotificationState>((set, get) => ({
  notifications: [],

  unreadCount: () => get().notifications.filter((n) => !n.read).length,

  addNotification: (notification) =>
    set((state) => ({
      notifications: [notification, ...state.notifications],
    })),

  markAllRead: () =>
    set((s) => ({
      notifications: s.notifications.map((n) => ({
        ...n,
        read: true,
      })),
    })),

  markRead: (id) =>
    set((s) => ({
      notifications: s.notifications.map((n) =>
        n.id === id ? { ...n, read: true } : n,
      ),
    })),

  clearAll: () => set({ notifications: [] }),
}));
