// SPDX-License-Identifier: AGPL-3.0-or-later
import {defineConfig} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import basicSsl from '@vitejs/plugin-basic-ssl';

// The server embeds dist/ with include_bytes!, so file names must be stable
// and the bundle must satisfy its CSP: no inline scripts, styles or hosts.
const upstream = process.env.CCP_PANEL_UPSTREAM ?? 'https://127.0.0.1:8443';

export default defineConfig({
  base: '/admin/',
  plugins: [tailwindcss(), svelte(), basicSsl()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    cssCodeSplit: false,
    modulePreload: {polyfill: false},
    assetsInlineLimit: 0,
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        codeSplitting: false,
        entryFileNames: 'app.js',
        chunkFileNames: 'app-[name].js',
        assetFileNames: asset => {
          const name = asset.names?.[0] ?? '';
          if (name.endsWith('.css')) return 'app.css';
          if (name.endsWith('.woff2')) return 'fonts/[name][extname]';
          return '[name][extname]';
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': {
        target: upstream,
        secure: false,
        changeOrigin: true,
        configure: proxy => {
          // The server binds CSRF proof to its own origin.
          proxy.on('proxyReq', request => request.setHeader('origin', upstream));
        },
      },
    },
  },
});
