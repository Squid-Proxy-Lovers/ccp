// SPDX-License-Identifier: AGPL-3.0-or-later
// Execute the merged access actions with their real model and view scopes.
// Only the Svelte app dependency is injected; no node_modules are required.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {stripTypeScriptTypes} from 'node:module';
import {LinkAttempt} from '../../crates/server/web/src/lib/link_issuance.js';
import {parseEnrollmentLink} from '../../crates/server/web/src/lib/enrollment_link.js';
import {viewScope} from '../../crates/server/web/src/lib/viewScope.js';
import {acquireDraft, dropDraft} from '../../crates/server/web/src/lib/drafts.js';

const url = new URL('../../crates/server/web/src/views/access/actions.ts', import.meta.url);
let source = await readFile(url, 'utf8');
source = source.replace(/import \{app as panelApp, type ViewApp\} from '[^']+';/, 'let panelApp;');
source = source.replace(/from '([^']+)'/g, (_, path) => `from '${new URL(path, url).href}'`);
source = stripTypeScriptTypes(source, {mode: 'transform'}) + '\nexport function bindApp(app) { panelApp = app; }';
const {changeLink, FormDraft, bindApp} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const vectors = JSON.parse(await readFile(new URL('../../crates/protocol/testdata/enrollment_links.json', import.meta.url)));
const uri = vectors.cases[0].uri.replaceAll('{invitation}', 'ccp_join_' + vectors.invitation_suffix);
const link = parseEnrollmentLink(uri);

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return {promise, resolve, reject};
}

function harness(group = false) {
  const notices = [], reports = [], writes = [];
  const workspace = {id: link.workspace_id, name: link.workspace_name, archived: false};
  const state = {
    session: {instance_id: link.instance_id, csrf_token: 'original-proof', identity: {
      key_id: 'key', principal_id: 'owner', scope: {kind: 'workspace', workspace_id: workspace.id, role: 'owner'},
    }},
    sessionEpoch: 0, accessWrites: 0, accessSecret: null, workspace, pending: null, dirty: false, drafts: {}, locked: false,
    verify: async () => state.session,
    clean() { this.dirty = false; },
    lock(message) { this.locked = true; this.lockMessage = message; },
    formDraft(key) { return acquireDraft(this.drafts, key); },
    dropFormDraft(key) { dropDraft(this.drafts, key); },
  };
  const reply = {invitation_id: link.workspace_id, workspace_id: workspace.id, workspace_name: workspace.name,
    max_uses: group ? 2 : 1, expires_at: Math.floor(Date.now() / 1000) + 1800, enrollment_link: uri};
  const transport = {write: async (...args) => { writes.push(args); return reply; }};
  const effects = {effect: fn => fn(), notice: (...args) => notices.push(args),
    wire: async (...args) => args[0] === '/api/v1/admin/session' ? state.verify() : transport.write(...args)};
  state.forSession = () => viewScope(state, effects);
  bindApp(state);
  const app = viewScope(state, effects);
  const form = new FormDraft('link', {role: 'read', mode: 'short'});
  form.set('https', link.https_origin);
  const enrollment = group ? {workspace_id: workspace.id, phase: 'unused', closed_at: null,
    has_admission_history: false, workspace_archived: false, can_open: true, invitation: null} : null;
  const attempt = new LinkAttempt(state.session, workspace, 'read', {capacity: '2'}, enrollment, group);
  const parent = viewScope(state, effects);
  const run = ask => changeLink(app, value => parent.effect(() => reports.push(value)), attempt, () => form.drop(), ask);
  return {app, parent, state, form, attempt, transport, reply, notices, reports, writes, run};
}

for (const group of [false, true]) for (const transition of ['unmount', 'lock']) {
  test(`${group ? 'group' : 'individual'} link receipt survives ${transition} during the dispatched write`, async () => {
    const h = harness(group), gate = deferred();
    h.transport.write = async () => gate.promise;
    const running = h.run(); await new Promise(resolve => setImmediate(resolve));
    assert.ok(h.state.pending); const outstanding = h.state.accessWrites;
    h.app.dispose();
    if (transition === 'lock') { h.parent.dispose(); h.state.lock('Concurrent lock'); }
    gate.resolve(h.reply); await running;
    assert.equal(h.state.pending, null); assert.equal(h.form.touched, false); assert.equal(h.state.accessWrites, 0);
    assert.equal(outstanding, 1);
    assert.equal(h.state.accessSecret.value, uri); assert.equal(h.state.accessSecret.kind, 'enrollment_link');
    if (transition === 'lock') { assert.match(h.state.lockMessage, /creation confirmed/); assert.deepEqual(h.reports, []); }
    else assert.equal(h.reports[0].phase, 'done');
  });
}

for (const group of [false, true]) test(`merged ${group ? 'group' : 'individual'} issuance cleans its draft before postflight and shows the link once`, async () => {
  const h = harness(group), postflight = deferred();
  let verifies = 0;
  h.state.verify = async () => { if (++verifies === 2) return postflight.promise; return h.state.session; };
  const running = h.run();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(verifies, 2); assert.equal(h.form.touched, false); assert.equal(h.state.pending, null);
  assert.deepEqual(h.reports, []); assert.equal(h.writes.length, 1);
  postflight.resolve(h.state.session); await running;
  assert.equal(h.reports.length, 1); assert.equal(h.reports[0].phase, 'done'); assert.equal(h.state.accessSecret.value, uri);
  assert.equal(h.notices.length, 1); assert.throws(() => h.attempt.request(h.state.session), /Do not retry/);
});

test('postflight denial keeps acknowledged fields out of locked recovery and withholds the private link', async () => {
  const h = harness(); let verifies = 0;
  h.state.verify = async () => { if (++verifies === 2) throw new Error('Expired'); return h.state.session; };
  await h.run();
  assert.equal(h.state.locked, true); assert.match(h.state.lockMessage, /creation confirmed/);
  assert.equal(h.form.touched, false); assert.equal(h.state.pending, null);
  assert.deepEqual(h.reports, []); assert.deepEqual(h.notices, []);
});

for (const malformed of [false, true]) test(`${malformed ? 'unverifiable' : 'lost'} link response retains the exact pending request without an unsent draft`, async () => {
  const h = harness();
  h.transport.write = async () => { if (!malformed) throw new Error('Lost response'); return {...h.reply, max_uses: 99}; };
  await h.run();
  assert.equal(h.state.pending, h.attempt); assert.equal(h.form.touched, false);
  assert.equal(h.reports[0].phase, 'pending'); assert.equal(h.reports[0].attempt.requestBody, h.attempt.requestBody);
  assert.throws(() => h.attempt.request(h.state.session), /Do not retry/);
});

test('declined confirmation and preflight denial retain the unsent fields', async () => {
  const h = harness(true);
  assert.equal(await h.run(async () => false), false);
  assert.equal(h.form.touched, true); assert.equal(h.writes.length, 0);
  h.state.verify = async () => { throw new Error('Preflight denied'); };
  await assert.rejects(h.run(), /Preflight denied/);
  assert.equal(h.form.touched, true); assert.equal(h.state.pending, null); assert.deepEqual(h.reports, []);
});

for (const stage of ['preflight', 'write', 'postflight']) for (const replacement of ['view', 'session']) {
  test(`retiring the ${replacement} during ${stage} keeps operation facts session-bound while preserving independent drafts`, async () => {
    const h = harness(), gate = deferred(); let verifies = 0;
    h.state.verify = async () => {
      verifies++;
      if (stage === 'preflight' && verifies === 1 || stage === 'postflight' && verifies === 2) return gate.promise;
      return h.state.session;
    };
    if (stage === 'write') h.transport.write = async () => gate.promise;
    const running = h.run().then(() => null, error => error);
    await new Promise(resolve => setImmediate(resolve));
    if (replacement === 'view') h.app.dispose();
    else { h.state.session = structuredClone(h.state.session); h.state.sessionEpoch++; }
    const key = replacement === 'view' && stage !== 'preflight' ? 'other' : 'link';
    h.state.formDraft(key).https = 'replacement draft';
    gate.resolve(stage === 'write' ? h.reply : h.state.session);
    const failure = await running;
    if (stage === 'preflight') assert.match(failure.message, /superseded/i);
    else assert.equal(failure, null);
    assert.equal(h.state.drafts[key].https, 'replacement draft');
    assert.equal(h.state.locked, false);
    if (replacement === 'view' && stage !== 'preflight') {
      assert.equal(h.state.pending, null); assert.equal(h.state.accessSecret.value, uri);
      assert.equal(h.reports[0].phase, 'done'); assert.equal(h.notices.length, 1);
      assert.equal('link' in h.state.drafts, false);
    } else { assert.deepEqual(h.reports, []); assert.deepEqual(h.notices, []); }
    if (stage === 'preflight') assert.equal(h.writes.length, 0);
  });
}

for (const stage of ['preflight', 'postflight']) for (const replacement of ['view', 'session']) {
  test(`a failed ${stage} check respects the ${replacement} boundary after acknowledgement`, async () => {
    const h = harness(), gate = deferred(); let verifies = 0;
    h.state.verify = async () => {
      if (++verifies === (stage === 'preflight' ? 1 : 2)) return gate.promise;
      return h.state.session;
    };
    const running = h.run().catch(error => error);
    await new Promise(resolve => setImmediate(resolve));
    if (replacement === 'view') h.app.dispose();
    else { h.state.session = structuredClone(h.state.session); h.state.sessionEpoch++; }
    h.state.formDraft('link').https = 'replacement draft';
    gate.reject(new Error('Retired session check failed'));
    await running;
    assert.equal(h.state.locked, replacement === 'view' && stage === 'postflight');
    if (h.state.locked) {
      assert.match(h.state.lockMessage, /Enrollment link creation confirmed/);
      assert.equal(h.state.accessSecret.value, uri); assert.equal(h.state.pending, null);
    }
    assert.deepEqual(h.notices, []); assert.deepEqual(h.reports, []);
    assert.equal(h.state.drafts.link.https, 'replacement draft');
  });
}
