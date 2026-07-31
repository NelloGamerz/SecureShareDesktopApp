import { create } from "zustand";
import { persist } from "zustand/middleware";

type Theme = "dark" | "light";

interface UIState {
  sidebarCollapsed: boolean;
  mobileSidebarOpen: boolean;
  commandOpen: boolean;
  theme: Theme;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setMobileSidebarOpen: (open: boolean) => void;
  setCommandOpen: (open: boolean) => void;
  toggleTheme: () => void;
  setTheme: (theme: Theme) => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      mobileSidebarOpen: false,
      commandOpen: false,
      theme: "dark",

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
    }),
    {
      name: "helix-ui",
      partialize: (s) => ({
        sidebarCollapsed: s.sidebarCollapsed,
        theme: s.theme,
      }),
    },
  ),
);
