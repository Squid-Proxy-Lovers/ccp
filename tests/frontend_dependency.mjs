// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import {createRequire} from 'node:module';
import {readFileSync} from 'node:fs';
import {dirname, join} from 'node:path';
import vm from 'node:vm';
import {mergeCases} from './frontend_merge_cases.mjs';

// Resolve from the consuming plugin, including any nested installation. A clean
// npm audit alone cannot establish that the actual executed package is patched.
const web = createRequire(new URL('../crates/server/web/package.json', import.meta.url));
const plugin = createRequire(web.resolve('@sveltejs/vite-plugin-svelte'));
const entry = plugin.resolve('deepmerge');
const sandbox = vm.createContext({module: {exports: {}}});
sandbox.exports = sandbox.module.exports;
vm.runInContext(readFileSync(join(dirname(dirname(entry)), 'dist/umd.js'), 'utf8'), sandbox);
const artifacts = [
  {name: 'installed CJS', merge: plugin('deepmerge'), prototype: Object.prototype},
  {name: 'published UMD', merge: sandbox.module.exports, prototype: vm.runInContext('Object.prototype', sandbox)},
];
for (const artifact of artifacts) {
  for (const [name, check] of Object.entries(mergeCases)) {
    test(artifact.name + ': ' + name, () => check(artifact.merge, artifact.prototype));
  }
}
