// SPDX-License-Identifier: AGPL-3.0-or-later
// Execute App's actual methods without node_modules. Node strips TypeScript;
// identity $state functions stand in for rune fields. This tests imperative
// state transitions, not Svelte reactivity, rendering, focus or DOM behavior.
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';

export const runtime = {toasts: [], confirmation: null};
const dataModule = source => 'data:text/javascript;base64,' + Buffer.from(source).toString('base64');
const helper = JSON.stringify(import.meta.url);
const notify = dataModule(`import {runtime} from ${helper};
  export function notice(text, tone) { runtime.toasts.push({text, tone}); }
  export function clearToasts() { runtime.toasts.length = 0; }
  export function bindNoticeScope() {}
  export async function copyText(value, notify = notice) { notify('Copied. Keep it private.', 'success'); return true; }`);
const confirm = dataModule(`import {runtime} from ${helper};
  export function confirmAction(options) {
    return new Promise(resolve => { runtime.confirmation = {...options, resolve}; });
  }`);

export async function panelAppClass() {
  const url = new URL('../../crates/server/web/src/lib/app.svelte.ts', import.meta.url);
  let source = stripTypeScriptTypes(await readFile(url, 'utf8'));
  source = source.replace(/from '([^']+)'/g, (match, specifier) => {
    if (specifier === './notify.svelte') return `from '${notify}'`;
    if (specifier === './ui/confirm.svelte') return `from '${confirm}'`;
    return specifier.startsWith('.') ? `from '${new URL(specifier, url).href}'` : match;
  });
  source = 'const $state = Object.assign(value => value, {raw: value => value});\n' + source;
  const {app} = await import(dataModule(source));
  return app.constructor;
}

export function answerConfirmation(ok) {
  const current = runtime.confirmation;
  if (!current) throw new Error('No confirmation is open.');
  runtime.confirmation = null;
  current.resolve(ok);
}

/** Exercise the real clipboard notification code without Svelte or a DOM. */
export async function panelNotifyModule() {
  const url = new URL('../../crates/server/web/src/lib/notify.svelte.ts', import.meta.url);
  const source = stripTypeScriptTypes(await readFile(url, 'utf8'));
  return import(dataModule('const $state = value => value;\n' + source));
}
