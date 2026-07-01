import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // 固定绑定 127.0.0.1 是为了让 Tauri 的 devUrl 与 Vite 实际监听地址完全一致，避免 Windows 上 localhost 被解析到 IPv6 或代理地址后连接超时。
    // Binding to 127.0.0.1 keeps Tauri's devUrl identical to Vite's real listener and avoids Windows localhost timeouts caused by IPv6 or proxy resolution.
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**"],
    },
  },
});
