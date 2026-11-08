// SPDX-License-Identifier: AGPL-3.0-or-later
import {defineConfig} from 'vite';
import {svelte} from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

// Cargo embeds the source build from OUT_DIR/panel (or an explicit prebuild),
// so file names must be stable
// and the bundle must satisfy its CSP: no inline scripts, styles or hosts.
// `npm run dev` proxies the admin API to a running CCP server.
const upstream = process.env.CCP_PANEL_UPSTREAM ?? 'http://127.0.0.1:1338';

export default defineConfig({
  base: '/admin/',
  plugins: [tailwindcss(), svelte()],
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
      '/v1': {target: upstream, changeOrigin: true},
    },
  },
});
