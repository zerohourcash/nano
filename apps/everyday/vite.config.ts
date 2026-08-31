import path from "path"
const __dirname = import.meta.dirname
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"
import { inspectAttr } from 'plugin-inspect-react-code'

export default defineConfig({
  plugins: [...(process.env.MK_OUT ? [] : [inspectAttr()]), react()],
  server: {
    port: 3000,
    host: true,
    proxy: {
      // changeOrigin оставлен выключенным намеренно: узел сверяет Origin с Host,
      // и подмена Host на 127.0.0.1:8080 ломала бы все мутации в dev-режиме.
      "/api": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
      },
      "/sync": {
        target: "http://127.0.0.1:8080",
        changeOrigin: false,
      },
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@contracts": path.resolve(__dirname, "./contracts"),
      "@db": path.resolve(__dirname, "./db"),
      "db": path.resolve(__dirname, "./db"),
    },
  },
  envDir: path.resolve(__dirname),
  build: {
    outDir: process.env.MK_OUT || path.resolve(__dirname, "dist/public"),
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks: {
          'react-vendor': ['react', 'react-dom', 'react-router'],
          'data-vendor': [
            '@tanstack/react-query',
            '@trpc/client',
            '@trpc/react-query',
            'superjson',
          ],
          'motion-vendor': ['framer-motion'],
        },
      },
    },
  },
});
