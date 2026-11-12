// SPDX-License-Identifier: AGPL-3.0-or-later
import {vitePreprocess} from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {runes: true, css: 'external'},
};
