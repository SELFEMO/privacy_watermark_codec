import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

const DEFAULT_DEV_PORT = 1420;
const rawDevPort = process.env.PWC_DEV_PORT;
const parsedDevPort = rawDevPort ? Number(rawDevPort) : DEFAULT_DEV_PORT;

if (!Number.isInteger(parsedDevPort) || parsedDevPort < 1 || parsedDevPort > 65535) {
  throw new Error(`Invalid PWC_DEV_PORT value: ${rawDevPort}`);
}

const devHost = process.env.PWC_DEV_HOST?.trim() || "127.0.0.1";
const launchedByTauri = process.env.PWC_TAURI_DEV === "1";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: parsedDevPort,
    // Tauri 启动前已经探测并同步了端口，因此这里必须禁止 Vite 再次静默换端口，否则 WebView 会连接到错误的 devUrl；单独运行 Vite 时仍允许自动回退。
    // Tauri preselects and synchronizes the port before startup, so Vite must not silently change it or the WebView would use the wrong devUrl; standalone Vite may still fall back automatically.
    strictPort: launchedByTauri,
    // 固定回环地址可避免 Windows 把 localhost 解析到 IPv6、代理或非预期网卡，同时不会向局域网暴露开发服务器。
    // A fixed loopback address avoids Windows resolving localhost through IPv6, a proxy, or an unexpected adapter without exposing the development server to the LAN.
    host: devHost,
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**"],
    },
  },
});
