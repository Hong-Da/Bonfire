import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri 开发服务器约定：固定端口 1420，只监听本机。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    // 文案文件与 Rust 共用，位于 crates/bonfire-app/locales。
    fs: { allow: [".", "../crates/bonfire-app/locales"] },
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
  },
});
