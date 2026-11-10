// SPDX-License-Identifier: AGPL-3.0-or-later
// Exercise real access actions with deferred transport and actual view/session scopes.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {viewScope} from '../../crates/server/web/src/lib/viewScope.js';
import {verifySession} from '../../crates/server/web/src/lib/sessionRequests.js';

const url = new URL('../../crates/server/web/src/views/access/actions.ts', import.meta.url);
let source = await readFile(url, 'utf8');
source = source.replace(/import \{app as panelApp, type ViewApp\} from '[^']+';/, 'let panelApp;');
source = source.replace(/from '([^']+)'/g, (_, path) => `from '${new URL(path, url).href}'`);
source = stripTypeScriptTypes(source, {mode: 'transform'}) + '\nexport function bindApp(app) { panelApp = app; }';
const actions = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const id = '11111111-1111-4111-8111-111111111111';
const secret = 'ccp_inv_' + 'a'.repeat(64);
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return {promise, resolve, reject};
}
function harness(kind) {
  const gate = deferred(), checking = deferred(), reports = [], notices = [];
  const state = {session: {instance_id: id, csrf_token: 'proof', identity: {key_id: 'original-key', principal_id: 'owner',
    scope: {kind: 'workspace', workspace_id: id, role: 'owner'}}}, sessionEpoch: 0, generation: 0, locked: false,
    pending: null, dirty: true, draft: 'submitted fields', accessSecret: null, accessWrites: 0,
    clean() { this.dirty = false; this.draft = ''; }, lock(message) { this.locked = true; this.lockMessage = message; },
  };
  let verifies = 0, settleCount = 0, writes = 0;
  const transport = {postflight: null, failure: null};
  const wire = async path => {
    if (path === '/api/v1/admin/session') {
      if (++verifies === 2) { checking.resolve(); if (transport.postflight) return transport.postflight.promise; }
      if (transport.failure) throw transport.failure;
      return state.session;
    }
    writes++;
    await gate.promise;
    if (transport.failure) throw transport.failure;
    // Unexpected fields from other endpoint shapes must never change secret handling.
    return {invitation_id: id, key_id: id, principal_id: 'admin', secret, enrollment_link: 'unverified-extra-field'};
  };
  const effects = {wire, notice: (...args) => notices.push(args), effect: fn => fn()};
  state.verify = () => verifySession(state, {wire, lock: message => state.lock(message)});
  state.forSession = () => viewScope(state, effects);
  actions.bindApp(state);
  const app = viewScope(state, effects), parent = state.forSession();
  const report = outcome => parent.effect(() => reports.push(outcome));
  const settled = () => { settleCount++; };
  const workspace = {id, name: 'Workspace', archived: false};
  const client = {client_id: id, client_disabled: false, revoked_at: null, client_name: 'Client'};
  const enrollment = {workspace_id: id, phase: 'unused', closed_at: null, has_admission_history: false,
    workspace_archived: false, can_open: true, invitation: null};
  const run = () => kind === 'admission'
    ? actions.changeAdmission(app, report, workspace, 'individual', {workspace_id: id, role: 'read', expires_at: 2000000000}, settled)
    : kind === 'enrollment'
      ? actions.changeEnrollment(app, report, enrollment, 'initial', {workspace_id: id, role: 'read', max_clients: 2, expires_at: null}, true, async () => true, settled)
      : kind === 'client'
        ? actions.changeClientAccess(app, report, workspace, client, {action: 'recover', expires_at: 2000000000}, async () => true, settled)
        : actions.changeSecurity(app, report, 'keys/issue', {workspace_id: id, principal_id: 'admin', name: 'Key', expires_at: null}, settled);
  return {state, app, parent, run, gate, checking, transport, reports, notices,
    get settled() { return settleCount; }, get writes() { return writes; }};
}
const turn = () => new Promise(resolve => setImmediate(resolve));
for (const kind of ['admission', 'enrollment', 'client', 'security']) {
  for (const transition of ['unmount', 'lock']) test(`acknowledged ${kind} settles after ${transition} and retains its one-time secret`, async () => {
    const h = harness(kind), running = h.run(); await turn();
    assert.equal(h.writes, 1); assert.ok(h.state.pending);
    if (transition === 'unmount') h.app.dispose(); else h.state.lock('Concurrent session lock');
    h.gate.resolve(); await running;
    assert.equal(h.state.pending, null); assert.equal(h.state.dirty, false); assert.equal(h.settled, 1);
    assert.equal(h.state.accessSecret?.value, secret); assert.equal(h.state.accessWrites, 0);
    if (transition === 'unmount') assert.equal(h.reports[0]?.phase, 'done');
    else assert.match(h.state.lockMessage, /confirmed|issued/i);
  });
  test(`acknowledged ${kind} publishes and cleans before deferred postflight verification`, async () => {
    const h = harness(kind); h.transport.postflight = deferred();
    const running = h.run(); await turn(); h.gate.resolve(); await h.checking.promise;
    assert.equal(h.state.accessSecret?.value, secret); assert.equal(h.settled, 1); assert.equal(h.state.pending, null);
    h.transport.postflight.resolve(h.state.session); await running;
  });
  test(`retired ${kind} identity cannot publish a secret or clear replacement work`, async () => {
    const h = harness(kind), running = h.run().catch(error => error); await turn();
    assert.equal(h.state.accessWrites, 1, 'locked-screen identity changes must wait for settlement');
    h.state.session = structuredClone(h.state.session); h.state.sessionEpoch++;
    const replacement = {body: 'replacement work'}; h.state.pending = replacement; h.state.dirty = true;
    h.gate.resolve(); await running;
    assert.equal(h.state.pending, replacement); assert.equal(h.state.dirty, true); assert.equal(h.state.accessSecret, null);
    assert.deepEqual(h.reports, []); assert.equal(h.settled, 0); assert.equal(h.state.accessWrites, 0);
  });
}
