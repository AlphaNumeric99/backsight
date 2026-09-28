import { useSyncExternalStore } from "react";
import type { Settings } from "@/ipc/api";

export type ThemePreference = Settings["theme"];
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "backsight.theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

function isPreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

/** The last theme the user chose, cached so the first paint already has the right colours. */
export function readCachedTheme(): ThemePreference {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return isPreference(value) ? value : "system";
  } catch {
    return "system";
  }
}

/** Sets `data-theme` on <html>; CSS does the rest (including following the OS for "system"). */
export function applyTheme(preference: ThemePreference): void {
  const root = document.documentElement;
  if (root.dataset.theme !== preference) root.dataset.theme = preference;
  try {
    localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Storage can be unavailable (private mode); the theme still applies for this session.
  }
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  if (preference !== "system") return preference;
  return typeof matchMedia === "function" && matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

function subscribe(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  const media = typeof matchMedia === "function" ? matchMedia(DARK_QUERY) : null;
  media?.addEventListener("change", onChange);
  return () => {
    observer.disconnect();
    media?.removeEventListener("change", onChange);
  };
}

function snapshot(): ResolvedTheme {
  const pref = document.documentElement.dataset.theme;
  return resolveTheme(isPreference(pref) ? pref : "system");
}

/** The theme actually on screen; re-renders when the preference or the OS scheme changes. */
export function useResolvedTheme(): ResolvedTheme {
  return useSyncExternalStore(subscribe, snapshot, () => "light");
}

/** Subscribes to theme changes outside React (the canvas timeline re-reads its palette). */
export const onThemeChange = subscribe;
