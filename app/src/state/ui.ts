import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

interface UiState {
  /** `null` = automatic (collapsed on narrow windows). */
  sidebarCollapsed: boolean | null;
  setSidebarCollapsed: (collapsed: boolean | null) => void;

  /** Selected tab on the Cameras page: "all", "favorites" or a group id. */
  homeGroup: string;
  setHomeGroup: (group: string) => void;

  addCameraOpen: boolean;
  setAddCameraOpen: (open: boolean) => void;

  multiviewPage: number;
  setMultiviewPage: (page: number) => void;
}

/** App-wide UI state that isn't server data. */
export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: null,
      setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
      homeGroup: "all",
      setHomeGroup: (homeGroup) => set({ homeGroup }),
      addCameraOpen: false,
      setAddCameraOpen: (addCameraOpen) => set({ addCameraOpen }),
      multiviewPage: 0,
      setMultiviewPage: (multiviewPage) => set({ multiviewPage }),
    }),
    {
      name: "backsight.ui",
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({ sidebarCollapsed: s.sidebarCollapsed, homeGroup: s.homeGroup }),
    },
  ),
);
