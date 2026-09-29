/// <reference types="vitest/config" />
import { rmSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

/**
 * The mock backend streams video fixtures from `public/fixtures/` (~1.5 MB). The desktop
 * app never uses the mock, so leave them out of release builds (the harness keeps them).
 */
const dropDevFixtures = (): Plugin => ({
  name: "backsight:drop-dev-fixtures",
  apply: "build",
  closeBundle() {
    if (!process.env.BACKSIGHT_HARNESS) {
      rmSync(fileURLToPath(new URL("./dist/fixtures", import.meta.url)), { recursive: true, force: true });
    }
  },
});

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss(), dropDevFixtures()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },

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
    // The app loads its bundle from disk, not the network, so one ~1 MB chunk is fine.
    chunkSizeWarningLimit: 1500,
    rolldownOptions: {
      input: {
        main: "index.html",
        // The player dev harness: `vite` serves it at /player-harness.html. It is left out of
        // app builds unless BACKSIGHT_HARNESS=1 (e.g. to try it with `vite preview`).
        ...(process.env.BACKSIGHT_HARNESS ? { harness: "player-harness.html" } : {}),
      },
    },
  },

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    restoreMocks: true,
  },
}));
