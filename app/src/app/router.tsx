import { createHashHistory, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { isLocalDate } from "@/lib/time";
import { RootLayout } from "./shell";
import { NotFoundPage } from "./not-found";
import { HomePage } from "@/features/home/home-page";
import { CameraPage } from "@/features/camera/camera-page";
import { MultiviewPage } from "@/features/multiview/multiview-page";
import { DownloadsPage } from "@/features/downloads/downloads-page";
import { SettingsPage } from "@/features/settings/settings-page";

export interface CameraSearch {
  tab?: "live" | "playback";
  /** Camera-local date shown in Playback, "YYYY-MM-DD". */
  date?: string;
  /** Where Playback starts, epoch ms. */
  t?: number;
}

const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFoundPage,
});

const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomePage,
});

const cameraRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/cameras/$cameraId",
  validateSearch: (search: Record<string, unknown>): CameraSearch => ({
    tab: search.tab === "playback" || search.tab === "live" ? search.tab : undefined,
    date: isLocalDate(search.date) ? search.date : undefined,
    t: typeof search.t === "number" && Number.isFinite(search.t) ? search.t : undefined,
  }),
  component: CameraPage,
});

const multiviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/multiview",
  component: MultiviewPage,
});

const downloadsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/downloads",
  component: DownloadsPage,
});

export type SettingsSection = "appearance" | "recordings" | "cameras" | "groups" | "about";

export interface SettingsSearch {
  /** Scrolls to a section on open. */
  section?: SettingsSection;
}

const SETTINGS_SECTIONS: SettingsSection[] = ["appearance", "recordings", "cameras", "groups", "about"];

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  validateSearch: (search: Record<string, unknown>): SettingsSearch => ({
    section: SETTINGS_SECTIONS.includes(search.section as SettingsSection)
      ? (search.section as SettingsSection)
      : undefined,
  }),
  component: SettingsPage,
});

export const routeTree = rootRoute.addChildren([
  homeRoute,
  cameraRoute,
  multiviewRoute,
  downloadsRoute,
  settingsRoute,
]);

export function createAppRouter() {
  // Hash history works everywhere the app is served from, including Tauri's asset protocol.
  return createRouter({ routeTree, history: createHashHistory(), defaultPreload: false });
}

export const router = createAppRouter();

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
