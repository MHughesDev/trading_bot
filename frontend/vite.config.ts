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
    proxy: {
      // Rust platform (REST + WebSocket) — see README quickstart.
      '/api': 'http://127.0.0.1:8080',
      '/ws': { target: 'ws://127.0.0.1:8080', ws: true },
      '/auth': 'http://127.0.0.1:8080',
      // Asset lifecycle + chart bars are served by the Rust platform (8080),
      // not the legacy Python backend (8001).
      '/assets': 'http://127.0.0.1:8080',
      '/trade': 'http://127.0.0.1:8001',
      '/portfolio': 'http://127.0.0.1:8001',
      '/positions': 'http://127.0.0.1:8001',
      '/pnl': 'http://127.0.0.1:8001',
      '/status': 'http://127.0.0.1:8001',
      '/routes': 'http://127.0.0.1:8001',
      '/models': 'http://127.0.0.1:8001',
      '/params': 'http://127.0.0.1:8001',
      '/universe': 'http://127.0.0.1:8001',
      '/strategies': 'http://127.0.0.1:8001',
      '/system': 'http://127.0.0.1:8001',
      '/governance': 'http://127.0.0.1:8001',
      '/flatten': 'http://127.0.0.1:8001',
      '/microservices': 'http://127.0.0.1:8001',
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
