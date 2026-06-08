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
      '/auth': 'http://127.0.0.1:8001',
      '/assets': 'http://127.0.0.1:8001',
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
