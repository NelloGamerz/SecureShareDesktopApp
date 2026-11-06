import { create } from "zustand";
import { persist } from "zustand/middleware";

type Theme = "dark" | "light";

interface UIState {
  sidebarCollapsed: boolean;
  mobileSidebarOpen: boolean;
  commandOpen: boolean;
  theme: Theme;
  autoAcceptTransfers: boolean;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setMobileSidebarOpen: (open: boolean) => void;
  setCommandOpen: (open: boolean) => void;
  toggleTheme: () => void;
  setTheme: (theme: Theme) => void;
  setAutoAcceptTransfers: (enabled: boolean) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      mobileSidebarOpen: false,
      commandOpen: false,
      theme: "dark",
      autoAcceptTransfers: true,

      toggleSidebar: () =>
        set((s) => ({
          sidebarCollapsed: !s.sidebarCollapsed,
        })),

      setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),

      setMobileSidebarOpen: (open) => set({ mobileSidebarOpen: open }),

      setCommandOpen: (open) => set({ commandOpen: open }),

      toggleTheme: () =>
        set((s) => ({
          theme: s.theme === "dark" ? "light" : "dark",
        })),

      setTheme: (theme) => set({ theme }),

      setAutoAcceptTransfers: (enabled) =>
        set({ autoAcceptTransfers: enabled }),
    }),
    {
      name: "vilsend-ui",
      partialize: (s) => ({
        sidebarCollapsed: s.sidebarCollapsed,
        theme: s.theme,
        autoAcceptTransfers: s.autoAcceptTransfers,
      }),
    },
  ),
);
