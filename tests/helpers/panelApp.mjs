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

/** Execute a trusted production callback with injected DOM/clock dependencies. */
export async function componentCallback(path, from, to, context, invoke = '') {
  const {runInNewContext} = await import('node:vm');
  const source = await readFile(new URL('../../crates/server/web/src/' + path, import.meta.url), 'utf8');
  const start = source.indexOf(from), end = source.indexOf(to, start + from.length);
  if (start < 0 || end < 0) throw new Error('Component callback markers changed: ' + path);
  return runInNewContext(source.slice(start, end) + '\n' + invoke, context);
}

/** Baseline adapter: the old view loaders applied results/errors without tickets. */
export async function readTaskClass() {
  try { return (await import('../../crates/server/web/src/lib/readTask.js')).ReadTask; }
  catch (error) {
    if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error;
    return class LegacyRead {
      constructor(app) { this.app = app; }
      async run(load, effects) {
        effects.start?.();
        try { const data = await load(); effects.receive(data); return data; }
        catch (error) { effects.error?.(error); if (effects.rethrow) throw error; }
        finally { effects.finish?.(); }
      }
    };
  }
}
