import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// Tauri 开发时前端跑在固定端口，构建产物直接内嵌进 exe
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: ['es2021', 'safari15'],
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
});
