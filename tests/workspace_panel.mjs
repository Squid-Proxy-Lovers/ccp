// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source = await readFile(new URL('../crates/server/src/workspace_http/panel/model.js', import.meta.url), 'utf8');
const {SecurityAttempt, canManageTeam, keyExpiry, Intent, GuidanceIntent, LifecycleAttempt, RetentionIntent, canChangeLifecycle, retentionSeconds, retentionPolicy, workspaceSnapshot, canPublish, guidanceBundle, signature, canWrite, exactRevision, takeFragment, wire} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

const session = () => ({instance_id: 'instance', csrf_token: 'proof', identity: {
  key_id: 'key', principal_id: 'owner', scope: {kind: 'workspace', workspace_id: 'workspace', role: 'owner'},
}});

test('an ambiguous write keeps its exact JSON/UUID/proof and cannot cross any session binding', () => {
  const current = session();
  const change = {action: 'revise', entry_id: 'entry', expected_revision: 4, content: {text: 'original'}};
  const intent = new Intent(current, 'workspace', change, 'operation');
  change.content.text = 'edited after dispatch';
  const first = intent.request(current);
  assert.equal(JSON.parse(first.body).change.content.text, 'original');
  assert.equal(JSON.parse(first.body).operation_id, 'operation');
  assert.deepEqual(intent.request(structuredClone(current)), first);
  for (const alter of [s => {s.csrf_token = 'new SID';}, s => {s.identity.key_id = 'other';},
    s => {s.instance_id = 'other';}, s => {s.identity.principal_id = 'other';},
    s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const changed = session(); alter(changed);
    assert.notEqual(signature(changed), signature(current));
    assert.throws(() => intent.request(changed), /session changed/);
  }
  assert.throws(() => {intent.body = '{}';}, TypeError);
});

test('global reads do not borrow scoped writes; archives and foreign scopes cannot edit', () => {
  const s = session();
  assert.equal(canWrite(s, {id: 'workspace', archived: false}), true);
  assert.equal(canWrite(s, {id: 'workspace', archived: true}), false);
  assert.equal(canWrite(s, {id: 'foreign', archived: false}), false);
  s.identity.scope = {kind: 'instance'};
  assert.equal(canWrite(s, {id: 'workspace', archived: false}), false);
});

test('fragment is scrubbed before key use; malformed or duplicate parameters are not accepted', () => {
  for (const [hash, expected] of [['#key=private%2Bvalue', 'private+value'], ['#key=a&key=b', ''],
    ['#key=a&scope=global', ''], ['#unknown=a', ''], ['#%FF', '']]) {
    const calls = [];
    const key = takeFragment({hash}, {replaceState: (...args) => calls.push(args)});
    assert.equal(key, expected);
    assert.deepEqual(calls, [[null, '', '/admin']]);
    assert.ok(!JSON.stringify(calls).includes('private'));
  }
  assert.equal(takeFragment({hash: ''}, {replaceState: () => assert.fail('empty fragment')}), '');
});

test('unsafe numeric revisions are refused instead of silently rounding a CAS request', () => {
  for (const value of [0, -1, 1.5, NaN, Infinity, '1', 9007199254740992]) assert.throws(() => exactRevision(value));
  assert.equal(exactRevision(1), 1);
  assert.equal(exactRevision(Number.MAX_SAFE_INTEGER), Number.MAX_SAFE_INTEGER);
});

test('wire never retries ambiguous writes and never echoes server-controlled errors', async () => {
  const original = globalThis.fetch;
  let calls = 0;
  try {
    globalThis.fetch = async (_path, options) => {
      calls += 1;
      assert.equal(options.credentials, 'same-origin');
      assert.equal(options.redirect, 'error');
      assert.equal(options.headers['X-CCP-CSRF'], 'proof');
      assert.equal(options.body, 'original');
      throw new Error('PRIVATE_BODY');
    };
    await assert.rejects(wire('/api/v1/content/entries/mutate', 'original', 'proof'), error => {
      assert.match(error.message, /may have completed/);
      assert.ok(!error.message.includes('PRIVATE_BODY')); return true;
    });
    assert.equal(calls, 1);
    globalThis.fetch = async () => ({ok: false, status: 409, json: async () => ({error: {code: 'PRIVATE_BODY'}})});
    await assert.rejects(wire('/api/v1/content/entries/mutate', 'original', 'proof'), error => {
      assert.equal(error.status, 409); assert.ok(!error.message.includes('PRIVATE_BODY')); return true;
    });
    globalThis.fetch = async () => ({ok: true, status: 200, json: async () => ({api_version: 1, data: {revision: 2}})});
    assert.deepEqual(await wire('/api/v1/content/entries/mutate', 'original', 'proof'), {revision: 2});
  } finally { globalThis.fetch = original; }
});

test('guidance publishing follows the key scope and active workspace independently of read access', () => {
  const owner = session();
  const ws = {id: 'workspace', archived: false};
  const scope = {kind: 'workspace', workspace_id: 'workspace'};
  assert.equal(canPublish(owner, scope, ws), true);
  assert.equal(canPublish(owner, scope, {...ws, archived: true}), false);
  assert.equal(canPublish(owner, {kind: 'workspace', workspace_id: 'foreign'}, ws), false);
  assert.equal(canPublish(owner, {kind: 'global'}, ws), false);
  assert.equal(canPublish(owner, scope, null), false);
  const global = session(); global.identity.scope = {kind: 'instance'};
  assert.equal(canPublish(global, scope, ws), false);
  assert.equal(canPublish(global, {kind: 'global'}, {...ws, archived: true}), true);
});

test('guidance intent preserves empty/Unicode text and the exact original proof, nonce and scope', () => {
  const s = session();
  const scope = {kind: 'workspace', workspace_id: 'workspace'};
  const text = 'é'.repeat(4096);
  const intent = new GuidanceIntent(s, scope, 0, text, 'guidance-operation');
  scope.workspace_id = 'changed after dispatch';
  const first = intent.request(s);
  assert.equal(first.proof, 'proof');
  assert.equal(JSON.parse(first.body).text, text);
  assert.equal(JSON.parse(first.body).scope.workspace_id, 'workspace');
  assert.equal(JSON.parse(first.body).expected_revision, 0);
  assert.equal(JSON.parse(first.body).operation_id, 'guidance-operation');
  assert.deepEqual(intent.request(structuredClone(s)), first);
  for (const alter of [v => {v.csrf_token = 'new session';}, v => {v.identity.key_id = 'other key';}, v => {v.identity.scope.role = 'admin';}]) {
    const changed = session(); alter(changed); assert.throws(() => intent.request(changed), /session changed/);
  }
  assert.equal(JSON.parse(new GuidanceIntent(s, {kind: 'global'}, 3, '', 'empty').body).text, '');
  for (const bad of ['é'.repeat(4097), '😀'.repeat(2049)]) assert.throws(() => new GuidanceIntent(s, {kind: 'global'}, 0, bad));
  for (const revision of [-1, 1.5, NaN, 9007199254740992, Number.MAX_SAFE_INTEGER]) assert.throws(() => new GuidanceIntent(s, {kind: 'global'}, revision, 'text'));
});

test('guidance receipts must confirm the original board, UUID and exactly the next revision', () => {
  const intent = new GuidanceIntent(session(), {kind: 'global'}, 4, 'saved', 'op');
  const receipt = {scope: {kind: 'global'}, operation_id: 'op', revision: 5};
  assert.equal(intent.receipt(receipt), 5);
  for (const changed of [{...receipt, revision: 6}, {...receipt, revision: 9007199254740992},
    {...receipt, operation_id: 'different'}, {...receipt, scope: {kind: 'workspace', workspace_id: 'workspace'}}]) {
    assert.throws(() => intent.receipt(changed));
  }
  assert.throws(() => {intent.body = '{}';}, TypeError);
});

test('guidance snapshots keep global-first and exact target/revision/UTF8 limits', () => {
  const global = {scope: {kind: 'global'}, revision: 0, text: '', publisher: null};
  const workspace = {scope: {kind: 'workspace', workspace_id: 'workspace'}, revision: 2, text: 'current', publisher: null};
  const bundle = {precedence: 'global_first', global, workspace};
  assert.equal(guidanceBundle(bundle, 'workspace'), bundle);
  assert.throws(() => guidanceBundle(bundle, 'foreign'));
  assert.throws(() => guidanceBundle(bundle, null));
  assert.equal(guidanceBundle({...bundle, workspace: null}, null).global.revision, 0);
  assert.throws(() => guidanceBundle({...bundle, precedence: 'workspace_first'}, 'workspace'));
  assert.throws(() => guidanceBundle({...bundle, global: {...global, revision: 9007199254740992}}, 'workspace'));
  assert.throws(() => guidanceBundle({...bundle, global: {...global, text: 'é'.repeat(4097)}}, 'workspace'));
});

test('workspace lifecycle is owner or instance scoped, independently of active or archived state', () => {
  const s = session(), ws = {id: 'workspace', name: 'Workspace', archived: false};
  assert.equal(canChangeLifecycle(s, ws), true);
  assert.equal(canChangeLifecycle(s, {...ws, archived: true}), true);
  s.identity.scope.role = 'admin'; assert.equal(canChangeLifecycle(s, ws), false);
  s.identity.scope.role = 'owner'; assert.equal(canChangeLifecycle(s, {...ws, id: 'foreign'}), false);
  s.identity.scope = {kind: 'instance'}; assert.equal(canChangeLifecycle(s, ws), true);
  assert.equal(canChangeLifecycle(s, null), false);
});

test('retention duration requires a positive explicit whole amount and exact allowed units', () => {
  assert.equal(retentionSeconds('7', '86400'), 604800);
  assert.equal(retentionSeconds('2', '3600'), 7200);
  assert.equal(retentionSeconds('1', '1'), 1);
  for (const [value, unit] of [['', '1'], ['0', '1'], ['-1', '1'], ['1.5', '1'], ['1', '60'], ['9007199254740991', '86400'], ['Infinity', '1']]) {
    assert.throws(() => retentionSeconds(value, unit));
  }
});

test('retention reads never substitute manual policy for missing, foreign or unsafe metadata', () => {
  const policy = {workspace_id: 'workspace', revision: 0, retention_seconds: null, changed_at: null};
  assert.equal(retentionPolicy(policy, 'workspace'), policy);
  for (const bad of [{...policy, workspace_id: 'foreign'}, {...policy, retention_seconds: undefined},
    {...policy, retention_seconds: 0}, {...policy, changed_at: 1}, {...policy, revision: 1},
    {...policy, revision: 9007199254740992}, {...policy, revision: 1, changed_at: 1, retention_seconds: 9007199254740992}]) {
    assert.throws(() => retentionPolicy(bad, 'workspace'));
  }
  const current = {workspace: {id: 'workspace', name: 'Workspace', archived: true}};
  assert.equal(workspaceSnapshot(current, 'workspace').archived, true);
  assert.throws(() => workspaceSnapshot(current, 'foreign'));
  assert.throws(() => workspaceSnapshot({workspace: {...current.workspace, archived: 'true'}}, 'workspace'));
});

test('retention changes require explicit confirmation, alternating modes and immutable receipt-bound intents', () => {
  const s = session();
  const manual = {workspace_id: 'workspace', revision: 0, retention_seconds: null, changed_at: null};
  assert.throws(() => new RetentionIntent(s, manual, 3600, false));
  assert.throws(() => new RetentionIntent(s, manual, null, false));
  const intent = new RetentionIntent(s, manual, 3600, true, 'retention-op');
  manual.revision = 99;
  const request = intent.request(s), original = JSON.parse(request.body);
  assert.equal(original.expected_revision, 0);
  assert.equal(original.confirm_timed_purge, true);
  assert.equal(original.retention_seconds, 3600);
  assert.deepEqual(intent.request(structuredClone(s)), request);
  const other = session(); other.csrf_token = 'different session'; assert.throws(() => intent.request(other));
  const saved = {workspace_id: 'workspace', revision: 1, retention_seconds: 3600, changed_at: 123};
  assert.equal(intent.receipt({operation_id: 'retention-op', policy: saved}), saved);
  for (const value of [{operation_id: 'other', policy: saved},
    {operation_id: 'retention-op', policy: {...saved, revision: 2}},
    {operation_id: 'retention-op', policy: {...saved, workspace_id: 'foreign'}},
    {operation_id: 'retention-op', policy: {...saved, retention_seconds: 7200}}]) assert.throws(() => intent.receipt(value));
  assert.throws(() => new RetentionIntent(s, saved, 7200, true));
  assert.throws(() => new RetentionIntent(s, saved, null, true));
  const disable = new RetentionIntent(s, saved, null, false, 'disable');
  assert.equal(JSON.parse(disable.body).retention_seconds, null);
  assert.equal(JSON.parse(disable.body).confirm_timed_purge, false);
  assert.throws(() => new RetentionIntent(s, {...saved, revision: Number.MAX_SAFE_INTEGER}, null, false));
});

test('lifecycle requests freeze endpoint and target, have no pretend nonce and cannot be sent twice', () => {
  const s = session(), workspace = {id: 'workspace', name: 'Workspace', archived: false};
  const attempt = new LifecycleAttempt(s, workspace);
  workspace.archived = true;
  assert.equal(attempt.path, '/api/v1/admin/workspaces/archive');
  assert.equal(attempt.archived, true);
  assert.deepEqual(JSON.parse(attempt.body), {path: attempt.path, request: {workspace_id: 'workspace'}, target_archived: true});
  const changed = session(); changed.identity.key_id = 'other'; assert.throws(() => attempt.request(changed));
  assert.deepEqual(JSON.parse(attempt.request(s).body), {workspace_id: 'workspace'});
  assert.throws(() => attempt.request(s), /Do not retry/);
  assert.throws(() => {attempt.path = 'other';}, TypeError);
  const restore = new LifecycleAttempt(s, workspace);
  assert.equal(restore.path, '/api/v1/admin/workspaces/reactivate');
  assert.equal(restore.archived, false);
});


test('team administration is owner-only in the matching workspace, including archived security', () => {
  const s = session();
  assert.equal(canManageTeam(s, {id: 'workspace', archived: true}), true);
  assert.equal(canManageTeam(s, {id: 'other'}), false);
  s.identity.scope.role = 'admin';
  assert.equal(canManageTeam(s, {id: 'workspace'}), false);
  s.identity.scope = {kind: 'instance'};
  assert.equal(canManageTeam(s, {id: 'workspace'}), false);
});

test('optional expiry defaults to until revoked and refuses past, invalid or fractional seconds', () => {
  const now = Date.parse('2026-10-20T12:00:00Z');
  assert.equal(keyExpiry('', now), null);
  assert.equal(keyExpiry('2026-10-21T12:00:00Z', now), now / 1000 + 86400);
  for (const value of ['invalid', '2026-10-19T12:00:00Z', '2026-10-20T12:00:00Z', '2026-10-21T12:00:00.001Z']) assert.throws(() => keyExpiry(value, now));
});

test('security action is frozen and one-shot without an invented operation ID or safe retry', () => {
  const s = session();
  for (const action of ['team/appoint', 'team/revoke', 'keys/issue', 'keys/revoke']) {
    const body = {workspace_id: 'workspace', principal: {kind: 'new_principal', name: 'original'}};
    const a = new SecurityAttempt(s, action, body);
    body.principal.name = 'edited';
    assert.equal(JSON.parse(a.body).request.principal.name, 'original');
    assert.equal(a.path, '/api/v1/admin/' + action);
    assert.ok(!a.body.includes('operation_id'));
    assert.ok(!a.body.includes('proof'));
    assert.equal(a.request(s).proof, 'proof');
    assert.throws(() => a.request(s), /Do not retry/);
    assert.throws(() => {a.path = 'other';}, TypeError);
  }
  assert.throws(() => new SecurityAttempt(s, 'workspaces/transfer-owner', {workspace_id: 'workspace'}), /Unknown/);
});

test('security action cannot be dispatched under changed credentials, scope or session', () => {
  for (const mutate of [s => {s.csrf_token = 'new';}, s => {s.identity.key_id = 'other';},
    s => {s.instance_id = 'other';}, s => {s.identity.principal_id = 'other';},
    s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const s = session(), a = new SecurityAttempt(s, 'keys/revoke', {workspace_id: 'workspace', key_id: 'target'});
    const changed = structuredClone(s); mutate(changed);
    assert.throws(() => a.request(changed), /session changed/);
    assert.deepEqual(JSON.parse(a.request(s).body), {workspace_id: 'workspace', key_id: 'target'});
  }
  const s = session(); s.identity.scope.role = 'admin';
  assert.throws(() => new SecurityAttempt(s, 'keys/issue', {workspace_id: 'workspace'}), /owner key/);
});
