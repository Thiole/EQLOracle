import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import path from 'node:path';

const root = import.meta.dirname;

// Tauri expects a fixed dev-server port; failing hard on a taken port
// (rather than Vite's default "try the next one") keeps `devUrl` in
// tauri.conf.json trustworthy instead of silently pointing at nothing.
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(root, './src'),
      $lib: path.resolve(root, './src/lib'),
    },
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
});
