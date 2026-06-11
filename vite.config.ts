import path from "path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ mode }) => {
  // Vite respects a pre-existing NODE_ENV even for `vite build`. If the shell
  // has NODE_ENV=development, a `vite build` still emits a "production" bundle
  // but with dev-mode React (StrictMode double-rendering, +280 kB of eager JS).
  // Pin it so production builds always tree-shake to the minified React runtime.
  if (mode === "production") {
    process.env.NODE_ENV = "production";
  }

  const env = loadEnv(mode, process.cwd(), "");
  const host = process.env.TAURI_DEV_HOST;
  // Port is read from .env.local (VITE_PORT). If changed here, also update
  // devUrl in src-tauri/tauri.conf.json to match.
  const port = parseInt(env.VITE_PORT || "1420");

  return {
    plugins: [react(), tailwindcss()],
    clearScreen: false,
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
      dedupe: ["@codemirror/state", "@codemirror/view", "@codemirror/language"],
    },
    build: {
      rollupOptions: {
        output: {
          manualChunks(id) {
            if (id.includes("node_modules/react-dom") || id.includes("node_modules/react/")) {
              return "react";
            }
            if (
              id.includes("node_modules/@base-ui/react") ||
              id.includes("node_modules/lucide-react") ||
              id.includes("node_modules/class-variance-authority") ||
              id.includes("node_modules/clsx") ||
              id.includes("node_modules/tailwind-merge")
            ) {
              return "ui";
            }
            if (id.includes("node_modules/@tauri-apps/")) {
              return "tauri";
            }
          },
        },
      },
    },
    server: {
      port,
      strictPort: true,
      host: host || "127.0.0.1",
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: port + 1,
          }
        : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
