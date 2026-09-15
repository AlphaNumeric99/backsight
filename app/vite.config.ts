import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  // The player's media worker and audio worklet are ES modules.
  worker: { format: "es" as const },

  build: {
    rolldownOptions: {
      input: {
        main: "index.html",
        // The player dev harness: `vite` serves it at /player-harness.html. It is left out of
        // app builds unless BACKSIGHT_HARNESS=1 (e.g. to try it with `vite preview`).
        ...(process.env.BACKSIGHT_HARNESS ? { harness: "player-harness.html" } : {}),
      },
    },
  },
}));
