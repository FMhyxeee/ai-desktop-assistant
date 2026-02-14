import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Tauri 期望的配置
  server: {
    strictPort: true,
    watch: {
      // 3. 忽略热更新
      ignored: ['**/src-tauri/**'],
    },
  },
  // 为 Tauri 前端优化
  clearScreen: false,
})
