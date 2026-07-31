import { create } from 'zustand';
import type { Session } from '@/types/auth';

interface AuthStoreState {
  session: Session | null;
  setSession: (session: Session | null) => void;
}

export const useAuthStore = create<AuthStoreState>((set) => ({
  session: null,
  setSession: (session) => set({ session }),
}));
