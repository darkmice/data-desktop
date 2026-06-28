import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri desktop client frontend. Dev server fixed at 1420 (Tauri convention),
// build output goes to ./dist which tauri.conf.json points to as frontendDist.
export default defineConfig({
  // 相对路径资源:Tauri 用 tauri:// / file:// 协议加载,绝对 /assets 会 404。
  base: './',
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2021',
  },
});
