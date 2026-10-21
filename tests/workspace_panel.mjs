// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source = await readFile(new URL('../crates/server/src/workspace_http/panel/model.js', import.meta.url), 'utf8');
const {Intent, GuidanceIntent, canPublish, guidanceBundle, signature, canWrite, exactRevision, takeFragment, wire} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

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
