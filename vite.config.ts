import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5601,
    strictPort: false, // 端口被占时 +1 顺延（多实例开发）；实际端口由 scripts/tauri-dev.mjs 解析后覆盖到 tauri devUrl，勿绕过 npm run tauri dev 直连 npx tauri dev（会脏跑到第一实例的 vite 上）
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // src-tauri：Rust 代码变化由 tauri dev 自己处理；
      // .dev-data：运行期数据（SQLite 频繁写入 + 临时空间项目副本的整目录拷贝/删除），
      // 若被监视，首发/清空临时空间会触发 vite 强制整页 reload，表现为界面闪现并跳回列表第一个会话
      ignored: ["**/src-tauri/**", "**/.dev-data/**"],
    },
  },
});
