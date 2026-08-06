import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri injects this when running `tauri dev` on a physical device.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Tauri owns the terminal output; Vite clearing it hides Rust compile errors.
  clearScreen: false,

  server: {
    port: 1420,
    // Fail loudly instead of silently drifting to another port: tauri.conf.json
    // hardcodes 1420 as devUrl, so a fallback port produces a blank window.
    strictPort: true,
    host: host || false,
    // Spread rather than assign undefined: `exactOptionalPropertyTypes` treats
    // an explicit undefined as a real value, and `hmr` does not accept one.
    ...(host ? { hmr: { protocol: "ws" as const, host, port: 1421 } } : {}),
    watch: {
      // Rust rebuilds are driven by cargo, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    // WebView2 (Windows) and WKWebView (macOS) both handle modern output.
    target: "esnext",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
  },
});
