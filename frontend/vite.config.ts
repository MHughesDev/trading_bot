import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'path'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  server: {
    // Honour an externally assigned port so several dev servers can coexist.
    port: process.env.PORT ? Number(process.env.PORT) : 5173,
    strictPort: false,
    proxy: {
      // Rust platform (REST + WebSocket) — see README quickstart.
      '/api': 'http://127.0.0.1:7080',
      '/ws': { target: 'ws://127.0.0.1:7080', ws: true },
      '/auth': 'http://127.0.0.1:7080',
      // Asset lifecycle + chart bars are served by the Rust platform (7080),
      // not the legacy Python backend (8001).
      '/assets': 'http://127.0.0.1:7080',
      // Portfolio, execution, risk, alerts, settings and account activity all
      // moved onto the Rust platform (migration 0042), so they ride the /api
      // proxy above. The only routes still pointed at the legacy Python service
      // are the ones it alone serves.
      '/universe': 'http://127.0.0.1:8001',
      '/trade': 'http://127.0.0.1:8001',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
