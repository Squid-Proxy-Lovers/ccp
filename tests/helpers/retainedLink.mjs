// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {panelAppClass, runtime, answerConfirmation, componentCallback} from './panelApp.mjs';
import {LinkAttempt} from '../../crates/server/web/src/lib/link_issuance.js';
import {readFile} from 'node:fs/promises';
const App = await panelAppClass();
const id = '11111111-1111-4111-8111-111111111111';
const turn = () => new Promise(resolve => setImmediate(resolve));
function setup(t) {
  runtime.toasts.length = 0; runtime.confirmation = null;
  const app = new App();
  const session = {instance_id: id, csrf_token: 'proof', expires_at: 2000000000, identity: {key_id: 'key',
    principal_id: 'owner', principal_name: 'Owner', scope: {kind: 'workspace', workspace_id: id, role: 'owner'}}};
  app.session = session; app.workspace = {id, name: 'Workspace', archived: false}; app.view = 'access';
  app.booted = true; app.accessSecret = {kind: 'enrollment_link', title: 'Enrollment link', value: 'private enrollment link', workspace_id: id};
  app.inspectWorkspace = async () => {}; app.loadWorkspaces = async () => {};
  const savedFetch = globalThis.fetch, requests = [];
  globalThis.fetch = async (path, init) => {
    requests.push(path);
    const data = path === '/api/v1/admin/session/login'
      ? {...session, csrf_token: 'replacement-proof', identity: JSON.parse(init.body).key === 'same' ? session.identity : {...session.identity, key_id: 'new-key'}}
      : path === '/api/v1/admin/session/logout' ? {signed_out: true} : session;
    return {ok: true, status: 200, json: async () => ({api_version: 1, data})};
  };
  t.after(() => { if (runtime.confirmation) answerConfirmation(false); globalThis.fetch = savedFetch; });
  return {app, session, requests, secret: app.accessSecret};
}
for (const action of ['navigate', 'refreshView', 'openWorkspace']) test(`undismissed enrollment link requires a named ${action} decision and stays visible`, async t => {
  const h = setup(t), run = () => action === 'navigate' ? h.app.navigate('context') : action === 'refreshView'
    ? h.app.refreshView() : h.app.openWorkspace({id: 'other', name: 'Other', archived: false});
  const declined = run(); await turn();
  assert.match(runtime.confirmation?.body ?? '', /enrollment link/i);
  answerConfirmation(false); await declined;
  assert.equal(h.app.view, 'access'); assert.equal(h.app.workspace.id, id); assert.equal(h.app.accessSecret, h.secret);
  const accepted = run(); await turn(); assert.match(runtime.confirmation?.body ?? '', /enrollment link/i);
  answerConfirmation(true); await accepted; assert.equal(h.app.accessSecret, h.secret);
});
for (const action of ['signOut', 'hashKeySwitch', 'discardAndSignIn']) test(`${action} names the undismissed enrollment link and never carries it to another identity`, async t => {
  const h = setup(t);
  if (action === 'discardAndSignIn') h.app.lock('Session ended');
  const run = () => action === 'hashKeySwitch' ? h.app.hashKeySwitch('different') : h.app[action]();
  const declined = run(); await turn();
  assert.match(runtime.confirmation?.body ?? '', /enrollment link/i);
  assert.equal(h.requests.length, 0); answerConfirmation(false); await declined;
  assert.equal(h.app.accessSecret, h.secret); assert.equal(h.app.session, h.session);
  const accepted = run(); await turn(); assert.match(runtime.confirmation?.body ?? '', /enrollment link/i);
  answerConfirmation(true); await accepted;
  assert.equal(h.app.accessSecret, null);
  if (action === 'hashKeySwitch') assert.equal(h.app.session.identity.key_id, 'new-key');
  else assert.equal(h.app.session, null);
});
test('same-identity hash adoption asks about the enrollment link and preserves its exact receipt', async t => {
  const h = setup(t), running = h.app.hashKeySwitch('same'); await turn();
  assert.match(runtime.confirmation?.body ?? '', /enrollment link/i); answerConfirmation(true); await running;
  assert.equal(h.app.accessSecret, h.secret); assert.equal(h.app.session.identity.key_id, 'key');
  assert.equal(h.app.session.csrf_token, 'replacement-proof');
});
test('locked-screen sign-in waits for an access write before making a discard decision', async t => {
  const h = setup(t); h.app.accessSecret = null; h.app.accessWrites = 1; h.app.lock('Concurrent lock');
  await h.app.discardAndSignIn();
  assert.equal(h.app.session, h.session); assert.equal(h.app.waitingForWorkspaceCreate, true);
  assert.equal(runtime.confirmation, null);
});
test('beforeunload protects an undismissed enrollment link held without drafts', async t => {
  const h = setup(t), previous = {window: globalThis.window, location: globalThis.location, history: globalThis.history};
  const events = new EventTarget();
  globalThis.window = events; globalThis.location = {hash: ''}; globalThis.history = {replaceState() {}};
  t.after(() => Object.assign(globalThis, previous));
  h.app.activate = async () => {};
  await h.app.boot();
  const event = new Event('beforeunload', {cancelable: true}); events.dispatchEvent(event);
  assert.equal(event.defaultPrevented, true);
  h.app.accessSecret = null;
  const dismissed = new Event('beforeunload', {cancelable: true}); events.dispatchEvent(dismissed);
  assert.equal(dismissed.defaultPrevented, false);
});
test('Access read-back asks about the retained enrollment link before removing its local result', async () => {
  const decisions = [], effects = [];
  const app = {pending: null, dirty: false, approveAccessNavigation: async action => { decisions.push(action); return false; },
    inspectWorkspace: async () => effects.push('read'), clean: () => effects.push('clean'), effect: fn => fn()};
  await componentCallback('views/Access.svelte', '  async function reread()', '\n  const report',
    {app, notice() {}, confirmAction: async () => false, outcome: {phase: 'done'}, configuring: false, generation: 0,
      readGrants: async () => {}, grantsAfter: null, readWindow: async () => {}, tick: async () => {}, document: {}}, 'reread();');
  assert.equal(decisions.length, 1); assert.deepEqual(effects, []);
});
for (const name of ['InviteForm', 'InitialGroupForm']) test(`${name} freezes every field and discard while its own issuance is in flight`, async () => {
  const source = await readFile(new URL(`../../crates/server/web/src/views/access/${name}.svelte`, import.meta.url), 'utf8');
  assert.match(source, /const frozen = \$derived\(formFrozen\(app\.busy, app\.busyKey, '(?:invite|initial-group)-submit'\)\)/);
  const fieldset = source.slice(source.indexOf('<fieldset'), source.indexOf('</fieldset>'));
  assert.match(fieldset, /disabled=\{frozen\}/); assert.match(fieldset, /<LinkOptions/);
  assert.match(fieldset, /type="submit"/); assert.match(fieldset, /onclick=\{discard\}/);
  assert.match(source, /async function discard\(\) \{\s*if \(frozen\) return;/);
});
// Bypass browser required-attribute validation and invoke the real submit body.
// These assertions specifically kill the two surviving validator mutants.
for (const name of ['InitialGroupForm', 'InviteForm']) test(`${name} enforces ${name === 'InitialGroupForm' ? 'checkbox consent' : 'the pending-action guard'} in its submit code`, async () => {
  let running, calls = 0, confirmations = 0;
  const session = {instance_id: id, csrf_token: 'proof', identity: {key_id: 'key', principal_id: 'owner',
    scope: {kind: 'workspace', workspace_id: id, role: 'owner'}}};
  const app = {session, workspace: {id, name: 'Workspace', archived: false}, pending: name === 'InviteForm' ? {body: 'original'} : null,
    run: fn => { running = fn(); return running; }};
  const context = {$derived: value => value, formFrozen: () => false, app, form: {get: key => key === 'role' ? 'read' : '', drop() {}}, LinkAttempt, linkFields: () => ({}),
    enrollment: {workspace_id: id, phase: 'unused', closed_at: null, has_admission_history: false, workspace_archived: false, can_open: true, invitation: null},
    changeLink: async () => { calls++; }, onoutcome() {}, confirmAction: async () => { confirmations++; return true; }};
  await componentCallback(`views/access/${name}.svelte`, '  function submit(', '\n  async function discard()', context,
    'submit({preventDefault() {}});');
  await assert.rejects(running, name === 'InitialGroupForm' ? /Confirm the single initial group/ : /Resolve the original invitation action/);
  assert.equal(calls, 0); assert.equal(confirmations, 0);
});
