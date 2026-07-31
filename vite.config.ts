import path from 'path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Tauri v2 exposes env vars prefixed with TAURI_ for dev / build context.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  // Tauri requires a deterministic port and supports mobile via host.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: 'ws', host, port: 1421 }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    // Tauri desktop webview prefers smaller bundles and relative asset paths.
    target: 'es2021',
    sourcemap: !!process.env.TAURI_DEBUG,
  },
  optimizeDeps: {
    exclude: ['lucide-react'],
  },
});
