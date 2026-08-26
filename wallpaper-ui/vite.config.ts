import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Tauri serves this on a fixed port in development and reads dist/ in a
// release build.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5183, strictPort: true },
  build: { target: 'chrome110', outDir: 'dist' },
})
