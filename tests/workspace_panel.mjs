// SPDX-License-Identifier: AGPL-3.0-or-later
// Pure-function tests for the panel's behavioural decisions. model.js and
// creationConfirm.js are loaded as base64 data: URLs (an established pattern
// in this file, kept as-is); every other lib/*.js module here is imported
// directly by relative path -- plain ESM file imports, not a bundler and not
// node_modules, which is what lets draftCreate.js etc. import drafts.js
// normally instead of inlining everything to dodge data: URLs' inability to
// resolve relative specifiers.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const source = await readFile(new URL('../crates/server/web/src/lib/model.js', import.meta.url), 'utf8');
const {WorkspaceDeletionIntent, canDeleteWorkspace, deletionPreview, deletionLookup, deletionStatusRequest, deletionReceipt, deletionTracking, StatusClearIntent, reportedWorkStatus, reportedWorkPage, ContentAdminIntent, shelfState, archivedShelfPage, ClientAccessAttempt, clientRecoveryScope, recoveryStatus, recoveryPage, EnrollmentAttempt, enrollmentStatus, initialBounds, AdmissionAttempt, canManageInvitations, invitationDeadline, invitationStatus, invitationPage, TransferAttempt, canTransferOwnership, SecurityAttempt, principalIdentity, canManageTeam, keyExpiry, Intent, GuidanceIntent, LifecycleAttempt, RetentionIntent, canChangeLifecycle, retentionSeconds, retentionPolicy, workspaceSnapshot, canPublish, guidanceBundle, signature, canWrite, exactRevision, takeFragment, wire, isStrictUuid, identityKey} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

const creationConfirmSource = await readFile(new URL('../crates/server/web/src/views/context/creationConfirm.js', import.meta.url), 'utf8');
const {creationConfirmedText} = await import('data:text/javascript;base64,' + Buffer.from(creationConfirmSource).toString('base64'));

import {acquireDraft, dropDraft, dropDraftIfCurrent, isCurrentDraft, peekDraft, hasDraftContent, hasAnyDraftContent,
  draftLabels, draftsRecoveryParts, recoveryText, draftLabel, shelfBadgeLabel} from '../crates/server/web/src/lib/drafts.js';
import {WORKSPACE_DRAFT_KEY, shelfDraftKey, bookDraftKey} from '../crates/server/web/src/lib/draftKeys.js';
import {submitDraftCreate, reportCreateFailure} from '../crates/server/web/src/lib/draftCreate.js';
import {discardDraft} from '../crates/server/web/src/lib/draftDiscard.js';
import {identitySwitchInFlight, formFrozen, hashSwitchBlocked, unsavedWorkSummary, runSignOut, runLogin, runIdentitySwitch}
  from '../crates/server/web/src/lib/identitySwitch.js';
import {createWorkspaceFlow} from '../crates/server/web/src/lib/workspaceCreate.js';
import {focusHandoffKey} from '../crates/server/web/src/lib/focusHandoff.js';
import {hasUnsavedWork} from '../crates/server/web/src/lib/unsavedWork.js';
import {clearedSessionState} from '../crates/server/web/src/lib/sessionReset.js';

/** A promise whose resolution is controlled from outside, for simulating an in-flight request. */
function defer() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return {promise, resolve, reject};
}

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

test('rejected sign-in names the admin key while protected requests retain the session warning', async () => {
  const original = globalThis.fetch;
  let calls = 0;
  try {
    globalThis.fetch = async () => {
      calls += 1;
      return {ok: false, status: 401, json: async () => ({api_version: 1, error: {code: 'PRIVATE_SERVER_DETAIL'}})};
    };
    await assert.rejects(wire('/api/v1/admin/session/login', JSON.stringify({key: 'PRIVATE_ADMIN_KEY'}), 'login'), error => {
      assert.equal(error.status, 401);
      assert.match(error.message, /admin key was not accepted/);
      assert.doesNotMatch(error.message, /unsaved|PRIVATE_ADMIN_KEY|PRIVATE_SERVER_DETAIL/);
      return true;
    });
    assert.equal(calls, 1);
    await assert.rejects(wire('/api/v1/admin/workspaces'), error => {
      assert.equal(error.status, 401);
      assert.match(error.message, /no longer authorized\. Sign in again to continue/);
      assert.doesNotMatch(error.message, /PRIVATE_ADMIN_KEY|PRIVATE_SERVER_DETAIL/);
      return true;
    });
    assert.equal(calls, 2);
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


test('ownership transfer requires active matching owner and a different enabled current admin', () => {
  const s = session(), w = {id: 'workspace', archived: false}, a = {id: 'next', enabled: true, is_owner: false};
  assert.equal(canTransferOwnership(s, w, a), true);
  for (const changed of [{...a, enabled: false}, {...a, is_owner: true}, {...a, id: 'owner'}, {...a, id: ''}]) assert.equal(canTransferOwnership(s, w, changed), false);
  assert.equal(canTransferOwnership(s, {...w, archived: true}, a), false);
  assert.equal(canTransferOwnership(s, {...w, id: 'other'}, a), false);
  s.identity.scope.role = 'admin'; assert.equal(canTransferOwnership(s, w, a), false);
  s.identity.scope = {kind: 'instance'}; assert.equal(canTransferOwnership(s, w, a), false);
});

test('ownership transfer needs explicit delivery attestation and exact typed principal ID', () => {
  const s = session(), w = {id: 'workspace', archived: false}, a = {id: 'next', enabled: true, is_owner: false};
  for (const delivered of [false, null, 1, 'true']) assert.throws(() => new TransferAttempt(s, w, a, delivered, 'next'));
  for (const text of ['', 'Next', 'next ', 'other']) assert.throws(() => new TransferAttempt(s, w, a, true, text));
  const t = new TransferAttempt(s, w, a, true, 'next');
  assert.deepEqual(JSON.parse(t.request(s).body), {workspace_id: 'workspace', new_owner_id: 'next'});
});

test('ownership transfer freezes original target and is one-shot without nonce or transmitted attestations', () => {
  const s = session(), w = {id: 'workspace', archived: false}, a = {id: 'next', enabled: true, is_owner: false};
  const t = new TransferAttempt(s, w, a, true, 'next');a.id = 'edited';w.id = 'edited';
  const diagnostic = JSON.parse(t.body);
  assert.equal(diagnostic.former_owner_id, 'owner');
  assert.equal(diagnostic.request.new_owner_id, 'next');
  assert.equal(t.path, '/api/v1/admin/workspaces/transfer-owner');
  assert.ok(!t.body.includes('operation_id')&&!t.body.includes('proof')&&!t.body.includes('delivery_confirmed'));
  const first = t.request(s);assert.equal(first.proof, 'proof');assert.equal(JSON.parse(first.body).workspace_id, 'workspace');
  assert.throws(() => t.request(s), /Do not retry/);
  assert.throws(() => {t.body = '{}';}, TypeError);
});

test('ownership transfer cannot cross session credentials, instance, principal or role changes', () => {
  for (const mutate of [s => {s.csrf_token = 'new';}, s => {s.identity.key_id = 'other';},
    s => {s.instance_id = 'other';}, s => {s.identity.principal_id = 'other';},
    s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const s = session(), t = new TransferAttempt(s, {id: 'workspace', archived: false}, {id: 'next', enabled: true, is_owner: false}, true, 'next');
    const changed = structuredClone(s);mutate(changed);assert.throws(() => t.request(changed), /session changed/);
    assert.equal(JSON.parse(t.request(s).body).new_owner_id, 'next');
  }
});


test('invitation security scope allows matching owner/admin including archive, creation requires active', () => {
  const s = session(), w = {id: 'workspace', archived: true};
  for (const role of ['owner', 'admin']) {s.identity.scope.role=role;assert.equal(canManageInvitations(s,w),true);assert.throws(()=>new AdmissionAttempt(s,w,'individual',{workspace_id:'workspace',role:'read',expires_at:2000}));}
  assert.equal(canManageInvitations(s,{id:'other'}),false);
  s.identity.scope.role='unknown';assert.equal(canManageInvitations(s,w),false);
  s.identity.scope={kind:'instance'};assert.equal(canManageInvitations(s,w),false);
});

test('invitation deadlines and metadata reject unsafe or unknown values instead of defaulting', () => {
  assert.equal(invitationDeadline('24',1000000),87400);
  for(const hours of ['',0,-1,1.5,169,'x',Infinity])assert.throws(()=>invitationDeadline(hours,1000000));
  const status={invitation_id:'invite',uses:0,max_uses:1,expires_at:2000,revoked:false,expired:false,initial_closed:false,issuer_authorized:false,target_client_id:null};
  assert.deepEqual(invitationStatus(status,'invite'),status);
  for(const alter of [s=>{s.uses=Number.MAX_SAFE_INTEGER+1;},s=>{s.uses=-1;},s=>{s.max_uses=0;},s=>{s.uses=2;},s=>{s.expired=null;},s=>{delete s.issuer_authorized;},s=>{s.expires_at=-1;},s=>{s.target_client_id='';}]){const changed=structuredClone(status);alter(changed);assert.throws(()=>invitationStatus(changed));}
  assert.throws(()=>invitationStatus(status,'other'));
  assert.throws(()=>invitationPage({invitations:[],next_cursor:'unknown'}));
  const item={kind:'individual',role:'read',status};
  assert.deepEqual(invitationPage({invitations:[item],next_cursor:'invite'}).invitations,[item]);
  assert.throws(()=>invitationPage({invitations:[item,item],next_cursor:null}));
  assert.throws(()=>invitationPage({invitations:[{...item,role:'owner'}],next_cursor:null}));
  assert.throws(()=>invitationPage({invitations:[{...item,kind:'recovery'}],next_cursor:null}));
});

test('admission actions are one-shot with frozen original deadline/ID and no invented replay ID', () => {
  const s=session(),w={id:'workspace',archived:false};
  for(const [action,body] of [['individual',{workspace_id:'workspace',role:'read',expires_at:2000}],['revoke',{workspace_id:'workspace',invitation_id:'invite'}]]){
    const t=new AdmissionAttempt(s,w,action,body);body.expires_at=9999;body.invitation_id='changed';
    const first=t.request(s);assert.equal(first.proof,'proof');assert.equal(t.path,'/api/v1/admin/invitations/'+action);
    const original=JSON.parse(first.body);assert.equal(action==='individual'?original.expires_at:original.invitation_id,action==='individual'?2000:'invite');
    assert.ok(!t.body.includes('operation_id')&&!t.body.includes('proof'));
    assert.throws(()=>t.request(s),/Do not retry/);assert.throws(()=>{t.body='{}';},TypeError);
  }
  assert.throws(()=>new AdmissionAttempt(s,w,'initial',{workspace_id:'workspace'}));
  assert.throws(()=>new AdmissionAttempt(s,w,'individual',{workspace_id:'workspace',role:'owner',expires_at:2000}));
});

test('admission original cannot cross any session binding, while archived revocation remains permitted', () => {
  for(const mutate of [s=>{s.csrf_token='new';},s=>{s.identity.key_id='other';},s=>{s.instance_id='other';},s=>{s.identity.principal_id='other';},s=>{s.identity.scope.role='admin';},s=>{s.identity.scope.workspace_id='other';}]){
    const s=session(),t=new AdmissionAttempt(s,{id:'workspace',archived:true},'revoke',{workspace_id:'workspace',invitation_id:'invite'});
    const changed=structuredClone(s);mutate(changed);assert.throws(()=>t.request(changed),/session changed/);
    assert.equal(JSON.parse(t.request(s).body).invitation_id,'invite');
  }
});


const unusedWindow = () => ({workspace_id: 'workspace', phase: 'unused', closed_at: null,
  invitation: null, has_admission_history: false, workspace_archived: false, can_open: true});

test('initial-window metadata distinguishes unused, open and closed without inferring eligibility', () => {
  const unused = unusedWindow();
  assert.equal(enrollmentStatus(unused, 'workspace').can_open, true);
  const history = {...unused, has_admission_history: true, can_open: false};
  assert.equal(enrollmentStatus(history, 'workspace').phase, 'unused');
  assert.equal(enrollmentStatus({...unused, phase: 'closed', closed_at: 1000, can_open: false}, 'workspace').invitation, null);
  for (const changed of [{...unused, has_admission_history: true}, {...unused, workspace_archived: true},
    {...unused, phase: 'open', can_open: false}, {...unused, phase: 'closed', can_open: false},
    {...unused, workspace_id: 'other'}, {...unused, phase: 'unknown'}]) {
    assert.throws(() => enrollmentStatus(changed, 'workspace'));
  }
});

test('initial group bounds permit count or time or both, with no implicit default', () => {
  assert.deepEqual(initialBounds('2', '', 1000000), {max_clients: 2, expires_at: null});
  assert.deepEqual(initialBounds('', '2', 1000000), {max_clients: null, expires_at: 8200});
  assert.deepEqual(initialBounds('3', '2', 1000000), {max_clients: 3, expires_at: 8200});
  assert.equal(initialBounds('4294967295', '', 1000000).max_clients, 4294967295);
  for (const [capacity, hours] of [['', ''], ['0', ''], ['4294967296', ''], ['1.5', ''],
    ['', '0'], ['', '-1'], ['', '1.5'], ['', 'Infinity'], ['', '9999999999999999']]) {
    assert.throws(() => initialBounds(capacity, hours, 1000000));
  }
});

test('initial-window actions require explicit confirmation and freeze a one-shot original without nonce', () => {
  const s = session(), state = unusedWindow();
  const body = {workspace_id: 'workspace', role: 'read', max_clients: 2, expires_at: null};
  assert.throws(() => new EnrollmentAttempt(s, state, 'initial', body, false));
  assert.throws(() => new EnrollmentAttempt(s, state, 'initial', {...body, max_clients: null}, true));
  const attempt = new EnrollmentAttempt(s, state, 'initial', body, true);
  body.max_clients = 99;
  assert.equal(JSON.parse(attempt.request(s).body).max_clients, 2);
  assert.throws(() => attempt.request(s), /Do not retry/);
  assert.ok(!attempt.body.includes('operation_id') && !attempt.body.includes('proof'));
  const archived = {...unusedWindow(), workspace_archived: true, can_open: false};
  assert.throws(() => new EnrollmentAttempt(s, archived, 'initial', body, true));
  const close = new EnrollmentAttempt(s, archived, 'close', {workspace_id: 'workspace'}, true);
  assert.deepEqual(JSON.parse(close.request(s).body), {workspace_id: 'workspace'});
  assert.throws(() => new EnrollmentAttempt(s, {...state, phase: 'closed', closed_at: 1000, can_open: false}, 'close', body, true));
});

test('initial-window actions cannot cross original session, scope or principal bindings', () => {
  for (const mutate of [s => {s.csrf_token = 'new';}, s => {s.identity.key_id = 'other';},
    s => {s.instance_id = 'other';}, s => {s.identity.principal_id = 'other';},
    s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const s = session(), attempt = new EnrollmentAttempt(s, unusedWindow(), 'close', {workspace_id: 'workspace'}, true);
    const changed = structuredClone(s); mutate(changed);
    assert.throws(() => attempt.request(changed), /session changed/);
    assert.equal(JSON.parse(attempt.request(s).body).workspace_id, 'workspace');
  }
});


const liveClient = () => ({client_id: 'client', client_name: 'label', client_disabled: false, revoked_at: null});

test('client recovery scope follows the actual key ceiling, never a selected workspace role union', () => {
  const s = session(), w = {id: 'workspace', archived: true};
  assert.deepEqual(clientRecoveryScope(s, w), {kind: 'workspace', workspace_id: 'workspace'});
  assert.throws(() => clientRecoveryScope(s, {id: 'foreign'}));
  s.identity.scope = {kind: 'instance', workspace_id: 'workspace', role: 'owner'};
  assert.deepEqual(clientRecoveryScope(s, w), {kind: 'instance'});
  const global = new ClientAccessAttempt(s, w, {...liveClient(), revoked_at: 0}, {action: 'recover', expires_at: 2000});
  assert.equal(JSON.parse(global.request(s).body).scope.kind, 'instance');
  assert.throws(() => new ClientAccessAttempt(s, {id: 'workspace', archived: false}, {...liveClient(), revoked_at: 0}, {action: 'readmit', role: 'read', expires_at: 2000}));
});

test('recovery metadata binds the exact client and scope without unsafe or duplicated records', () => {
  const scope = {kind: 'workspace', workspace_id: 'workspace'};
  const status = {client_id: 'client', scope, expires_at: 2000, revoked: false, expired: false, redeemed: false, issuer_authorized: true};
  assert.deepEqual(recoveryStatus(status, 'client', scope), status);
  assert.throws(() => recoveryStatus(status, 'other', scope));
  assert.throws(() => recoveryStatus(status, 'client', {kind: 'instance'}));
  assert.throws(() => recoveryStatus({...status, expires_at: Number.MAX_SAFE_INTEGER + 1}, 'client', scope));
  assert.throws(() => recoveryStatus({...status, redeemed: null}, 'client', scope));
  const row = {invitation_id: 'token', status};
  assert.equal(recoveryPage({invitations: [row], next_cursor: 'token'}, 'client', scope).invitations.length, 1);
  assert.throws(() => recoveryPage({invitations: [row, row], next_cursor: null}, 'client', scope));
  assert.throws(() => recoveryPage({invitations: [], next_cursor: 'missing'}, 'client', scope));
});

test('recovery and readmission are distinct explicit requests with archive and disable boundaries', () => {
  const s = session(), w = {id: 'workspace', archived: true}, client = liveClient();
  const recovery = new ClientAccessAttempt(s, w, client, {action: 'recover', expires_at: 2000});
  assert.deepEqual(JSON.parse(recovery.request(s).body), {client_id: 'client', scope: {kind: 'workspace', workspace_id: 'workspace'}, expires_at: 2000});
  assert.throws(() => new ClientAccessAttempt(s, w, {...client, revoked_at: 0}, {action: 'recover', expires_at: 2000}));
  assert.throws(() => new ClientAccessAttempt(s, w, {...client, revoked_at: 0}, {action: 'readmit', role: 'read', expires_at: 2000}));
  const readmit = new ClientAccessAttempt(s, {...w, archived: false}, {...client, revoked_at: 0}, {action: 'readmit', role: 'read_write', expires_at: 2000});
  assert.deepEqual(JSON.parse(readmit.request(s).body), {workspace_id: 'workspace', client_id: 'client', role: 'read_write', expires_at: 2000});
  assert.throws(() => new ClientAccessAttempt(s, w, {...client, client_disabled: true}, {action: 'recover', expires_at: 2000}));
  const revoke = new ClientAccessAttempt(s, w, {...client, client_disabled: true}, {action: 'revoke_recovery', invitation_id: 'token'});
  assert.deepEqual(JSON.parse(revoke.request(s).body), {invitation_id: 'token'});
});

test('client access actions keep exact one-shot identity/scope/deadline and cannot cross session bindings', () => {
  const s = session(), w = {id: 'workspace', archived: false}, client = liveClient();
  const change = {action: 'recover', expires_at: 2000};
  const attempt = new ClientAccessAttempt(s, w, client, change); client.client_id = 'changed'; change.expires_at = 9999;
  assert.equal(JSON.parse(attempt.request(s).body).client_id, 'client');
  assert.equal(JSON.parse(attempt.requestBody).expires_at, 2000);
  assert.throws(() => attempt.request(s), /Do not retry/);
  assert.throws(() => {attempt.scope.workspace_id = 'other';}, TypeError);
  assert.ok(!attempt.body.includes('operation_id') && !attempt.body.includes('proof'));
  for (const mutate of [s => {s.csrf_token = 'new';}, s => {s.identity.key_id = 'other';},
    s => {s.instance_id = 'other';}, s => {s.identity.principal_id = 'other';},
    s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const current = session(), t = new ClientAccessAttempt(current, w, liveClient(), {action: 'recover', expires_at: 2000});
    const changed = structuredClone(current); mutate(changed); assert.throws(() => t.request(changed), /session changed/);
    assert.equal(JSON.parse(t.request(current).body).client_id, 'client');
  }
});

test('archive management freezes exact target, CAS, operation and session across deliberate retries', () => {
  const current = session(), workspace = {id: 'workspace', archived: false};
  const state = {workspace_id: 'workspace', shelf_id: 'shelf', revision: 5, archived: false};
  const intent = new ContentAdminIntent(current, workspace, state, 'shelf', '', 'operation');
  state.shelf_id = 'changed'; state.revision = 9; workspace.id = 'elsewhere';
  const request = intent.request(current);
  assert.deepEqual(JSON.parse(request.body), {workspace_id: 'workspace', operation_id: 'operation', expected_revision: 5, shelf_id: 'shelf', archived: true});
  assert.equal(intent.path, '/api/v1/admin/content/shelves/transition');
  assert.deepEqual(intent.request(structuredClone(current)), request);
  for (const alter of [s => {s.csrf_token = 'other';}, s => {s.identity.key_id = 'other';}, s => {s.instance_id = 'other';},
    s => {s.identity.principal_id = 'other';}, s => {s.identity.scope.role = 'admin';}, s => {s.identity.scope.workspace_id = 'other';}]) {
    const changed = session(); alter(changed); assert.throws(() => intent.request(changed), /session changed/);
  }
  assert.throws(() => {intent.body = '{}';}, TypeError);
});

test('shelf lifecycle cannot borrow global authority or archived workspace write access', () => {
  const current = session(), workspace = {id: 'workspace', archived: false};
  const target = {workspace_id: 'workspace', shelf_id: 'shelf', revision: 1, archived: true};
  assert.equal(JSON.parse(new ContentAdminIntent(current, workspace, target, 'shelf', '', 'op').body).archived, false);
  assert.throws(() => new ContentAdminIntent(current, {...workspace, archived: true}, target, 'shelf'), /active workspace/);
  for (const scope of [{kind: 'instance'}, {kind: 'workspace', workspace_id: 'foreign', role: 'owner'}, {kind: 'workspace', workspace_id: 'workspace', role: 'reader'}]) {
    const other = session(); other.identity.scope = scope;
    assert.throws(() => new ContentAdminIntent(other, workspace, target, 'shelf'), /owner\/admin/);
  }
  for (const revision of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '1']) assert.throws(() => shelfState({...target, revision}, 'workspace', 'shelf'), /exact numeric/);
  assert.throws(() => new ContentAdminIntent(current, workspace, {...target, revision: Number.MAX_SAFE_INTEGER}, 'shelf'), /exact next revision/);
  assert.throws(() => shelfState(target, 'workspace', 'foreign'), /requested target/);
});

test('shelf receipts bind historical operation and result without claiming current state', () => {
  const target = {workspace_id: 'workspace', shelf_id: 'shelf', revision: 4, archived: false};
  const intent = new ContentAdminIntent(session(), {id: 'workspace', archived: false}, target, 'shelf', '', 'operation');
  const receipt = {operation_id: 'operation', result: {...target, revision: 5, archived: true}};
  assert.equal(intent.receipt(receipt), receipt);
  for (const bad of [{...receipt, operation_id: 'other'}, {...receipt, result: {...receipt.result, revision: 6}},
    {...receipt, result: {...receipt.result, archived: false}}, {...receipt, result: {...receipt.result, shelf_id: 'foreign'}},
    {...receipt, result: {...receipt.result, workspace_id: 'foreign'}}]) assert.throws(() => intent.receipt(bad), /receipt|target/);
});

test('purge requires exact archived entry confirmation, permits archived workspace and verifies body-free proof', () => {
  const workspace = {id: 'workspace', archived: true};
  const entry = {workspace_id: 'workspace', id: 'entry', book_id: 'book', revision: 8, archived: true};
  const intent = new ContentAdminIntent(session(), workspace, entry, 'purge', 'entry', 'operation');
  assert.equal(intent.path, '/api/v1/admin/content/entries/purge'); assert.equal(intent.book_id, 'book');
  assert.deepEqual(JSON.parse(intent.body), {workspace_id: 'workspace', operation_id: 'operation', expected_revision: 8, entry_id: 'entry', confirm_purge: true});
  assert.throws(() => new ContentAdminIntent(session(), workspace, entry, 'purge', 'wrong'), /exact typed ID/);
  assert.throws(() => new ContentAdminIntent(session(), workspace, {...entry, archived: false}, 'purge', 'entry'), /individually archived/);
  assert.throws(() => new ContentAdminIntent(session(), workspace, {...entry, workspace_id: 'foreign'}, 'purge', 'entry'), /this workspace/);
  const receipt = {workspace_id: 'workspace', entry_id: 'entry', operation_id: 'operation', revision: 8, purged_at: 100};
  assert.equal(intent.receipt(receipt), receipt);
  for (const field of ['workspace_id', 'entry_id', 'operation_id', 'revision', 'purged_at']) assert.throws(() => intent.receipt({...receipt, [field]: 'wrong'}), /receipt/);
  assert.throws(() => intent.receipt({...receipt, purged_at: -1}), /receipt/);
});

test('archived shelf pages reject foreign, duplicate, unsafe or unbound cursor metadata', () => {
  const item = {id: 'shelf', workspace_id: 'workspace', name: '<script>', revision: 3};
  assert.equal(archivedShelfPage({items: [item], next_cursor: 'shelf'}, 'workspace').items[0].name, '<script>');
  for (const value of [{items: [item, item], next_cursor: null}, {items: [{...item, workspace_id: 'foreign'}], next_cursor: null},
    {items: [{...item, revision: Number.MAX_SAFE_INTEGER + 1}], next_cursor: null}, {items: [], next_cursor: 'shelf'},
    {items: [item], next_cursor: 'other'}, {items: Array(33).fill(item), next_cursor: null}]) assert.throws(() => archivedShelfPage(value, 'workspace'), /metadata|numeric|page|cursor/);
});

test('reported-work pages bind workspace, independent status slots, cursor and server-observed age', () => {
  const item = {workspace_id: 'workspace', status_id: 'status', client_id: 'client', client_name: 'registered name', client_disabled: false,
    revision: 2, agent: '<script>literal</script>', message: 'Investigating retries', updated_at: 1000, stale: false};
  const page = {items: [item], next_cursor: 'status', observed_at: 1299, stale_after_seconds: 300};
  assert.equal(reportedWorkPage(page, 'workspace'), page);
  for (const [now, stale] of [[1299, false], [1300, true], [999, true]]) {
    assert.equal(reportedWorkPage({...page, observed_at: now, items: [{...item, stale}]}, 'workspace').items[0].stale, stale);
  }
  for (const value of [{...page, items: [item, item]}, {...page, next_cursor: 'different'}, {...page, items: [], next_cursor: 'status'},
    {...page, observed_at: Number.MAX_SAFE_INTEGER + 1}, {...page, stale_after_seconds: 0}, {...page, items: Array(33).fill(item)},
    {...page, items: [{...item, workspace_id: 'foreign'}]}, {...page, items: [{...item, stale: true}]},
    {...page, items: [{...item, message: null}]}, {...page, items: [{...item, message: 'é'.repeat(2049)}]},
    {...page, items: [{...item, agent: 'é'.repeat(65)}]}, {...page, items: [{...item, revision: 1.5}]},
    {...page, items: [{...item, client_disabled: 'no'}]}, {...page, items: [{...item, updated_at: -1}]}]) assert.throws(() => reportedWorkPage(value, 'workspace'), /page|workspace|cursor|freshness|numeric/);
});

test('status message and optional label bounds preserve exact text without inventing an agent identity', () => {
  const item = {workspace_id: 'workspace', status_id: 'status', client_id: 'client', client_name: 'name', client_disabled: true,
    revision: 1, agent: null, message: 'é'.repeat(2048), updated_at: 1000, stale: true};
  const page = {items: [item], next_cursor: null, observed_at: 1300, stale_after_seconds: 300};
  assert.equal(reportedWorkPage(page, 'workspace').items[0].message, item.message);
  assert.equal(reportedWorkPage(page, 'workspace').items[0].agent, null);
  for (const bad of [{message: ''}, {message: '\0'}, {agent: ''}, {agent: '\0'}, {agent: 1}, {client_id: ''}, {client_name: ''}]) assert.throws(() => reportedWorkPage({...page, items: [{...item, ...bad}]}, 'workspace'), /workspace/);
});

test('server-accepted U+FEFF text and agent labels preserve the full page and its cursor', () => {
  for (const [message, agent] of [['\uFEFF', 'label'], ['work', '\uFEFF'], ['\uFEFF', '\uFEFF'], [' \uFEFF\n', '\uFEFF ']]) {
    const first = {workspace_id: 'workspace', status_id: 'status-1', client_id: 'client', client_name: 'name', client_disabled: false,
      revision: 1, agent, message, updated_at: 1000, stale: false};
    const later = {...first, status_id: 'status-2', message: 'Other client work remains readable', agent: null};
    const page = {items: [first, later], next_cursor: 'status-2', observed_at: 1000, stale_after_seconds: 300};
    const result = reportedWorkPage(page, 'workspace');
    assert.equal(result.items[0].message, message); assert.equal(result.items[0].agent, agent);
    assert.equal(result.items[1].message, later.message); assert.equal(result.next_cursor, 'status-2');
  }
});


const moderationTarget = () => {
  const workspace = {id: '11111111-1111-4111-8111-111111111111', archived: false};
  const s = session(); s.identity.scope.workspace_id = workspace.id;
  return {s, workspace, report: {workspace_id: workspace.id, status_id: '22222222-2222-4222-8222-222222222222',
    client_id: '33333333-3333-4333-8333-333333333333', revision: 7, message: 'private work'}};
};
const moderationOperation = '44444444-4444-4444-8444-444444444444';
const clearReceipt = intent => ({...JSON.parse(intent.body), client_id: intent.client_id, revision: 8, cleared: true, updated_at: 100});

test('scoped status clearing freezes the original target/CAS/UUID without claiming client publication', () => {
  const {s, workspace, report} = moderationTarget();
  const intent = new StatusClearIntent(s, workspace, report, moderationOperation);
  report.revision = 20; report.client_id = 'changed'; workspace.id = 'changed';
  const first = intent.request(s); const body = JSON.parse(first.body);
  assert.equal(body.expected_revision, 7); assert.equal(body.operation_id, moderationOperation);
  assert.deepEqual(Object.keys(body).sort(), ['expected_revision', 'operation_id', 'status_id', 'workspace_id']);
  assert.equal(intent.path, '/api/v1/admin/work/status/clear');
  assert.equal(intent.client_id, '33333333-3333-4333-8333-333333333333');
  assert.deepEqual(intent.request(structuredClone(s)), first);
  assert.throws(() => {intent.body = '{}';}, TypeError);
});
test('status clear cannot borrow instance, archived, foreign or unknown scoped roles', () => {
  for (const change of [v => {v.s.identity.scope = {kind: 'instance'};}, v => {v.workspace.archived = true;},
    v => {v.s.identity.scope.role = 'reader';}, v => {v.report.workspace_id = 'foreign';},
    v => {v.report.message = null;}, v => {v.report.revision = Number.MAX_SAFE_INTEGER;},
    v => {v.report.status_id = '00000000-0000-0000-0000-000000000000';}]) {
    const value = moderationTarget(); change(value);
    assert.throws(() => new StatusClearIntent(value.s, value.workspace, value.report, moderationOperation));
  }
  const {s, workspace, report} = moderationTarget(); s.identity.scope.role = 'admin';
  assert.equal(new StatusClearIntent(s, workspace, report, moderationOperation).kind, 'status_clear');
});
test('status clear exact retries retain the original session and reject every changed identity binding', () => {
  const {s, workspace, report} = moderationTarget(); const intent = new StatusClearIntent(s, workspace, report, moderationOperation);
  for (const change of [v => {v.csrf_token = 'other';}, v => {v.instance_id = 'other';}, v => {v.identity.key_id = 'other';},
    v => {v.identity.principal_id = 'other';}, v => {v.identity.scope.role = 'admin';}, v => {v.identity.scope.workspace_id = 'other';}]) {
    const other = structuredClone(s); change(other); assert.throws(() => intent.request(other), /session changed/);
  }
});
test('status clear receipts bind the original owner target operation revision and action', () => {
  const {s, workspace, report} = moderationTarget(); const intent = new StatusClearIntent(s, workspace, report, moderationOperation);
  const receipt = clearReceipt(intent); delete receipt.expected_revision;
  assert.deepEqual(intent.receipt(receipt), receipt);
  for (const [field, value] of [['workspace_id', 'other'], ['status_id', 'other'], ['client_id', 'other'],
    ['operation_id', 'other'], ['revision', 9], ['cleared', false], ['updated_at', -1], ['updated_at', 1.5]]) {
    assert.throws(() => intent.receipt({...receipt, [field]: value}), /receipt/);
  }
});
test('separate status observations preserve stale Unicode and cleared tombstones without invented observation times', () => {
  const {workspace, report} = moderationTarget();
  const row = {...report, client_name: 'client', client_disabled: true, message: '\ufeff', agent: '\ufeff', updated_at: 100, stale: true};
  assert.deepEqual(reportedWorkStatus(row, workspace.id), row);
  const cleared = {...row, message: null, agent: null};
  assert.deepEqual(reportedWorkStatus(cleared, workspace.id), cleared);
  assert.throws(() => reportedWorkStatus(cleared, workspace.id, false));
  assert.throws(() => reportedWorkStatus({...cleared, stale: false}, workspace.id));
  assert.throws(() => reportedWorkStatus({...cleared, agent: 'not cleared'}, workspace.id));
});


const deletionWs = '12345678-1111-2222-3333-123456789abc';
const deletionOp = '12345678-1111-2222-3333-123456789def';
const deletionSession = () => {
  const s = session(); s.identity.scope.workspace_id = deletionWs; return s;
};
const deletedWorkspace = () => ({id: deletionWs, name: 'archived context', archived: true});
const deletionBase = () => ({workspace_id: deletionWs, name: 'archived context', lifecycle_revision: 2});
const deletionResult = () => ({workspace_id: deletionWs, operation_id: deletionOp, lifecycle_revision: 3, started_at: 1000, completed_at: null});

test('workspace deletion requires archived owner/global role, exact UUID confirmation and a verified lifecycle preview', () => {
  const s = deletionSession(), w = deletedWorkspace(), p = deletionBase();
  assert.equal(canDeleteWorkspace(s, w), true);
  for (const alter of [x => {x.identity.scope.role = 'admin';}, x => {x.identity.scope.workspace_id = 'foreign';}]) {
    const other = deletionSession(); alter(other);
    assert.equal(canDeleteWorkspace(other, w), false);
    assert.throws(() => new WorkspaceDeletionIntent(other, w, p, deletionWs, true, deletionOp));
  }
  const global = deletionSession(); global.identity.scope = {kind: 'instance'};
  assert.equal(canDeleteWorkspace(global, w), true);
  assert.equal(canDeleteWorkspace(s, {...w, archived: false}), false);
  for (const value of [false, 'true', null]) assert.throws(() => new WorkspaceDeletionIntent(s, w, p, deletionWs, value, deletionOp));
  for (const value of ['wrong', deletionWs.toUpperCase(), '', ' '+deletionWs]) assert.throws(() => new WorkspaceDeletionIntent(s, w, p, value, true, deletionOp));
  for (const value of [0, -1, 1.5, '2', Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER+1]) assert.throws(() => deletionPreview({...p, lifecycle_revision: value}, w));
  for (const value of [{...p, workspace_id: deletionOp}, {...p, name: 'renamed'}]) assert.throws(() => deletionPreview(value, w));
  assert.throws(() => new WorkspaceDeletionIntent(s, w, p, deletionWs, true, '00000000-0000-0000-0000-000000000000'));
});

test('deletion intent freezes original proof/body and cannot cross any session even for the same key', () => {
  const s = deletionSession(), p = deletionBase();
  const intent = new WorkspaceDeletionIntent(s, deletedWorkspace(), p, deletionWs, true, deletionOp);
  const first = intent.request(s); p.lifecycle_revision = 9;
  assert.deepEqual(JSON.parse(first.body), {workspace_id: deletionWs, operation_id: deletionOp, expected_revision: 2, confirm_workspace_id: deletionWs});
  assert.deepEqual(intent.request(structuredClone(s)), first);
  for (const alter of [x => {x.csrf_token = 'new proof';}, x => {x.instance_id = 'other';}, x => {x.identity.key_id = 'other';}, x => {x.identity.principal_id = 'other';}, x => {x.identity.scope = {kind: 'instance'};}]) {
    const changed = deletionSession(); alter(changed); assert.throws(() => intent.request(changed), /session changed/);
  }
  assert.throws(() => {intent.body = '{}';}, TypeError);
});

test('deletion receipt distinguishes pending from complete and never replaces a known receipt with stale or foreign data', () => {
  const intent = new WorkspaceDeletionIntent(deletionSession(), deletedWorkspace(), deletionBase(), deletionWs, true, deletionOp);
  const pending = deletionReceipt({...deletionResult(), text: 'PRIVATE'}, intent);
  assert.equal(pending.completed_at, null); assert.equal('text' in pending, false);
  const complete = deletionReceipt({...deletionResult(), completed_at: 1001}, intent, pending);
  assert.equal(complete.completed_at, 1001);
  for (const bad of [{...deletionResult(), workspace_id: deletionOp}, {...deletionResult(), operation_id: deletionWs},
    {...deletionResult(), lifecycle_revision: 4}, {...deletionResult(), started_at: -1}, {...deletionResult(), started_at: '1000'},
    {...deletionResult(), started_at: Number.MAX_SAFE_INTEGER+1}, {...deletionResult(), completed_at: undefined},
    {...deletionResult(), completed_at: 999}, {...deletionResult(), completed_at: '1001'}]) assert.throws(() => deletionReceipt(bad, intent));
  for (const bad of [deletionResult(), {...complete, completed_at: 1002}, {...complete, started_at: 999}]) assert.throws(() => deletionReceipt(bad, intent, complete));
  assert.deepEqual(deletionReceipt(complete, intent, complete), complete);
});

test('new global session may inspect same-instance body-free tracking but cannot resend the original owner intent', () => {
  const owner = deletionSession(), intent = new WorkspaceDeletionIntent(owner, deletedWorkspace(), deletionBase(), deletionWs, true, deletionOp);
  const global = deletionSession(); global.csrf_token = 'global proof'; global.identity.scope = {kind: 'instance'};
  assert.throws(() => intent.request(global));
  assert.deepEqual(deletionStatusRequest(global, intent), {workspace_id: deletionWs, operation_id: deletionOp});
  assert.throws(() => deletionStatusRequest(owner, intent));
  assert.throws(() => deletionStatusRequest({...global, instance_id: 'other instance'}, intent));
  const lookup = deletionLookup(global, deletionWs, deletionOp);
  assert.equal(deletionReceipt(deletionResult(), lookup).lifecycle_revision, 3);
  for (const value of ['bad', deletionWs.toUpperCase(), '', '00000000-0000-0000-0000-000000000000']) assert.throws(() => deletionLookup(global, value, deletionOp));
  assert.throws(() => deletionLookup(owner, deletionWs, deletionOp));
  const exported = JSON.parse(deletionTracking(intent, deletionReceipt(deletionResult(), intent)));
  assert.equal(exported.instance_id, owner.instance_id); assert.equal(exported.request.expected_revision, 2);
  for (const value of ['proof', 'session', 'key_id', 'principal_id']) assert.equal(value in exported, false);
  assert.equal(JSON.stringify(exported).includes('global proof'), false);
  assert.equal('request' in JSON.parse(deletionTracking(lookup, null)), false);
});

import './workspace_native_panel.mjs';


test('existing principal UUIDs are canonical before freezing and verifying a key request', () => {
  const lower = 'abcdef01-2345-6789-abcd-ef0123456789';
  for (const value of [lower, lower.toUpperCase(), `  ${lower.toUpperCase()}  `]) {
    const principal = principalIdentity('existing', value);
    const attempt = new SecurityAttempt(session(), 'team/appoint', {workspace_id: 'workspace', principal});
    assert.equal(principal.principal_id, lower);
    assert.equal(JSON.parse(attempt.requestBody).principal.principal_id, lower);
    principal.principal_id = 'edited';
    assert.equal(JSON.parse(attempt.body).request.principal.principal_id, lower);
  }
  assert.deepEqual(principalIdentity('new_principal', '  Case Sensitive  '), {kind: 'new_principal', name: 'Case Sensitive'});
  for (const value of ['', 'partial', 'abcdef01-2345-6789-abcd-ef0123456789/other']) {
    assert.throws(() => principalIdentity('existing', value));
  }
  assert.throws(() => principalIdentity('other', lower));
});

test('the shared strict UUID check accepts only canonical lowercase, non-nil UUIDs', () => {
  const canonical = '12345678-1111-2222-3333-123456789abc';
  assert.equal(isStrictUuid(canonical), true);
  assert.equal(isStrictUuid(canonical.toUpperCase()), false);
  assert.equal(isStrictUuid('00000000-0000-0000-0000-000000000000'), false);
  assert.equal(isStrictUuid('not-a-uuid'), false);
  assert.equal(isStrictUuid(123), false);
  assert.equal(isStrictUuid(null), false);
});

test('identityKey ignores the CSRF/session token the real server rotates on every login', () => {
  const base = session();
  const rotated = {...base, csrf_token: 'a-completely-different-token'};
  assert.notEqual(signature(base), signature(rotated), 'signature() still cares about csrf_token (session-drift detection needs that)');
  assert.equal(identityKey(base), identityKey(rotated), 'identityKey() must not, since the server issues a fresh one on every login, same key or not');
});

test('identityKey distinguishes instance, key, principal and scope, each independently', () => {
  const base = session();
  assert.notEqual(identityKey(base), identityKey({...base, instance_id: 'other-instance'}));
  for (const mutate of [s => { s.identity = {...s.identity, key_id: 'other'}; },
    s => { s.identity = {...s.identity, principal_id: 'other'}; },
    s => { s.identity = {...s.identity, scope: {...s.identity.scope, role: 'admin'}}; }]) {
    const changed = session(); mutate(changed);
    assert.notEqual(identityKey(changed), identityKey(base));
  }
  const same = session();
  assert.equal(identityKey(same), identityKey(base));
});

test('a dropped form draft is actually gone; re-acquiring it returns a live, independent object', () => {
  const drafts = {};
  const first = acquireDraft(drafts, 'create-workspace');
  first.name = 'Orphan';
  assert.equal(drafts['create-workspace'], first);
  dropDraft(drafts, 'create-workspace');
  assert.equal('create-workspace' in drafts, false);
  // The caller's old reference is now orphaned: it keeps its stale value but no
  // longer reaches the live store, so edits to it would never resurface.
  assert.equal(first.name, 'Orphan');
  const second = acquireDraft(drafts, 'create-workspace');
  assert.notEqual(second, first);
  assert.deepEqual(second, {});
  second.name = 'Fresh';
  assert.equal(drafts['create-workspace'].name, 'Fresh');
  assert.equal(first.name, 'Orphan');
});

test('peeking a form draft never creates it, unlike acquiring', () => {
  const drafts = {};
  assert.deepEqual(peekDraft(drafts, 'context.book.ws.shelf'), {});
  assert.equal('context.book.ws.shelf' in drafts, false);
  acquireDraft(drafts, 'context.book.ws.shelf').name = 'Notes';
  assert.deepEqual(peekDraft(drafts, 'context.book.ws.shelf'), {name: 'Notes'});
});

test('draft content is blank-trimmed, so sign-out treats whitespace-only fields as nothing to lose', () => {
  assert.equal(hasDraftContent({name: '', description: ''}), false);
  assert.equal(hasDraftContent({name: '   ', description: '\n'}), false);
  assert.equal(hasDraftContent({name: 'Research', description: ''}), true);
  assert.equal(hasAnyDraftContent({}), false);
  assert.equal(hasAnyDraftContent({a: {name: ''}, b: {name: '  '}}), false);
  assert.equal(hasAnyDraftContent({a: {name: ''}, b: {name: 'Shelf'}}), true);
});

test('an acknowledged shelf or book create drops out of recovery text once its draft is dropped', () => {
  const drafts = {'context.shelf.ws': {name: 'Research', description: ''}, 'context.book.ws.shelf1': {name: '', description: ''}};
  const before = recoveryText(null, '', drafts);
  assert.match(before, /Research/);
  assert.match(before, /name: Research/);
  // The create for this shelf is now confirmed server-side; its draft is dropped
  // exactly like entryWrite.ts clears `app.pending` once a write is confirmed.
  dropDraft(drafts, 'context.shelf.ws');
  const after = recoveryText(null, '', drafts);
  assert.doesNotMatch(after, /Research/);
});

test('discarding one draft leaves other unsaved work visible in recovery text', () => {
  const drafts = {'context.book.ws.shelf1': {name: 'Second shelf book', description: ''}};
  const withNote = recoveryText(null, 'An unsaved append note', drafts);
  assert.match(withNote, /unsaved append note/);
  assert.match(withNote, /Second shelf book/);
  // Discarding the separate note (app.draft) must not also discard the kept
  // creation draft that recoveryText reports independently.
  const noteDiscarded = recoveryText(null, '', drafts);
  assert.doesNotMatch(noteDiscarded, /unsaved append note/);
  assert.match(noteDiscarded, /Second shelf book/);
  assert.deepEqual(draftsRecoveryParts({}), []);
});

test('an uncertain pending write takes recovery precedence over a plain text draft', () => {
  assert.equal(recoveryText({body: 'PENDING_INTENT'}, 'ignored draft text', {}), 'PENDING_INTENT');
  assert.equal(recoveryText(null, 'kept draft text', {}), 'kept draft text');
  assert.equal(draftLabel('context.book.11111111-1111-4111-8111-111111111111'), 'Context book draft');
});

test('a confirmed shelf or book create reads like entryWrite.ts: confirmed, named and not a retry invitation', () => {
  const shelf = creationConfirmedText('shelf', {id: 'shelf-1', name: 'Research'});
  assert.match(shelf, /^Shelf “Research” creation confirmed \(id shelf-1\)/);
  assert.match(shelf, /do not repeat the create/);
  const book = creationConfirmedText('book', {id: 'book-1', name: 'Notes'});
  assert.match(book, /^Book “Notes” creation confirmed \(id book-1\)/);
});

test('dropping one form draft never touches another form\'s draft', () => {
  const drafts = {};
  acquireDraft(drafts, 'create-workspace').name = 'Workspace draft';
  const book = acquireDraft(drafts, 'context.book.ws.shelf');
  book.name = 'Book draft';
  dropDraft(drafts, 'create-workspace');
  assert.equal('create-workspace' in drafts, false);
  assert.equal(drafts['context.book.ws.shelf'], book);
  assert.equal(drafts['context.book.ws.shelf'].name, 'Book draft');
});

test('dropDraftIfCurrent and isCurrentDraft only act on the exact object they were given', () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  assert.equal(isCurrentDraft(drafts, 'k', a), true);
  const stranger = {name: 'not the live one'};
  assert.equal(isCurrentDraft(drafts, 'k', stranger), false);
  assert.equal(dropDraftIfCurrent(drafts, 'k', stranger), false);
  assert.equal(drafts['k'], a, 'a mismatched identity must not drop the real draft');
  assert.equal(dropDraftIfCurrent(drafts, 'k', a), true);
  assert.equal('k' in drafts, false);
  assert.equal(dropDraftIfCurrent(drafts, 'k', a), false, 'dropping an already-gone draft reports nothing to drop');
});

test('draftLabels names only the drafts that currently have content', () => {
  const drafts = {'create-workspace': {name: 'Probe', owner: ''}, 'context.shelf.ws': {name: '', description: '  '}};
  assert.deepEqual(draftLabels(drafts), ['Create workspace draft']);
  assert.deepEqual(draftLabels({}), []);
});

// ---- draft keys: every reader of the same target must agree on the same key ----

test('shelf and book draft keys are stable, parameterised and distinct per target', () => {
  assert.equal(WORKSPACE_DRAFT_KEY, 'create-workspace');
  assert.equal(shelfDraftKey('ws-1'), 'context.shelf.ws-1');
  assert.equal(bookDraftKey('ws-1', 'shelf-1'), 'context.book.ws-1.shelf-1');
  assert.notEqual(bookDraftKey('ws-1', 'shelf-1'), bookDraftKey('ws-1', 'shelf-2'));
  assert.notEqual(shelfDraftKey('ws-1'), shelfDraftKey('ws-2'));
});

test('shelfBadgeLabel prefers New over a kept book draft, and never mutates its input', () => {
  assert.equal(shelfBadgeLabel(true, {name: 'kept'}), 'New');
  assert.equal(shelfBadgeLabel(true, {}), 'New');
  assert.equal(shelfBadgeLabel(false, {name: 'Notes', description: ''}), 'Book draft kept');
  assert.equal(shelfBadgeLabel(false, {name: '', description: '  '}), undefined);
  assert.equal(shelfBadgeLabel(false, {}), undefined);
});

// ---- the create-then-drop flow (bug: a replacement draft erased by a stale create) ----
// submitDraftCreate now performs the drop itself, directly against `drafts`,
// rather than through an injectable "drop"/"isCurrent" callback a caller (or
// a mutation) could silently neuter -- there is no such parameter any more.

test('submitDraftCreate drops the submitted draft on success, not whatever replaced it', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  const write = defer();
  const outcomePromise = submitDraftCreate({
    drafts, key: 'k', submitted: a,
    request: () => write.promise,
    confirmedText: () => { throw new Error('must not be called on a plain success'); },
  });
  // While the create for A is still in flight, discard it and type a fresh draft B.
  dropDraft(drafts, 'k');
  const b = acquireDraft(drafts, 'k'); b.name = 'B';
  write.resolve({id: 'created-a'});
  const outcome = await outcomePromise;
  assert.equal(outcome.ok, true);
  assert.deepEqual(outcome.result, {id: 'created-a'});
  // B must survive: the stale completion for A must not erase it.
  assert.equal(drafts['k'], b);
  assert.equal(drafts['k'].name, 'B');
});

test('submitDraftCreate drops the submitted draft on success when nothing raced it', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  const outcome = await submitDraftCreate({
    drafts, key: 'k', submitted: a,
    request: async () => ({id: 'created-a'}),
    confirmedText: () => '',
  });
  assert.equal(outcome.ok, true);
  assert.equal('k' in drafts, false);
});

test('submitDraftCreate reports whether a failed create\'s draft is still the one it submitted', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  const untouched = await submitDraftCreate({
    drafts, key: 'k', submitted: a,
    request: async () => { throw new Error('503'); },
    confirmedText: () => '',
  });
  assert.equal(untouched.ok, false);
  assert.equal(untouched.keptCurrent, true, 'nothing raced it, so claiming the draft is kept is true');
  assert.equal(drafts['k'], a, 'a failed write never drops the draft');

  // Now repeat with a discard racing the same failing write.
  const write = defer();
  const racedPromise = submitDraftCreate({
    drafts, key: 'k', submitted: a,
    request: () => write.promise,
    confirmedText: () => '',
  });
  dropDraft(drafts, 'k');
  write.reject(new Error('503'));
  const raced = await racedPromise;
  assert.equal(raced.ok, false);
  assert.equal(raced.keptCurrent, false, 'the draft was already discarded, so it is false to call it kept');
});

test('submitDraftCreate\'s confirmed callback drops the draft and uses confirmedText, before the caller\'s lock throws', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  let seenData = null;
  const outcome = await submitDraftCreate({
    drafts, key: 'k', submitted: a,
    // Mirrors app.request(): calls `confirmed` with the write's result, then
    // throws with its text, when the post-write session check fails.
    request: async confirmed => { const text = confirmed({id: 'created-a', name: 'A'}); seenData = text; throw new Error(text); },
    confirmedText: data => `confirmed ${data.id} (${data.name})`,
  });
  assert.equal(seenData, 'confirmed created-a (A)');
  assert.equal('k' in drafts, false, 'the confirmed callback drops the draft before the request rejects');
  assert.equal(outcome.ok, false);
  assert.equal(outcome.error.message, 'confirmed created-a (A)');
});

test('submitDraftCreate\'s confirmed callback is also identity-safe: a draft replaced first is not dropped', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  dropDraft(drafts, 'k');
  const b = acquireDraft(drafts, 'k'); b.name = 'B';
  const outcome = await submitDraftCreate({
    drafts, key: 'k', submitted: a,
    request: async confirmed => { const text = confirmed({id: 'created-a'}); throw new Error(text); },
    confirmedText: () => 'confirmed',
  });
  assert.equal(outcome.ok, false);
  assert.equal(drafts['k'], b, 'B must survive a confirmed callback for the stale submission A');
});

// ---- the locked-or-failed create message: one place that decides it, not a Svelte branch ----

test('reportCreateFailure says nothing once the page is already locked', () => {
  let notified = false;
  reportCreateFailure({
    locked: true, outcome: {error: new Error('x'), keptCurrent: true}, kind: 'shelf',
    message: () => 'x', outcomeUnknown: () => true, notify: () => { notified = true; },
  });
  assert.equal(notified, false, 'the lock message already covers it');
});

test('reportCreateFailure names the draft kept only when it actually still is, with the uncertain-outcome hint when relevant', () => {
  const seen = [];
  const notify = (text, tone) => seen.push([text, tone]);
  reportCreateFailure({locked: false, outcome: {error: 'e', keptCurrent: true}, kind: 'shelf', message: () => 'Server unavailable.', outcomeUnknown: () => true, notify});
  assert.match(seen[0][0], /^Server unavailable\. Your shelf draft is kept\. Refresh the list/);
  assert.equal(seen[0][1], 'destructive');
  seen.length = 0;
  reportCreateFailure({locked: false, outcome: {error: 'e', keptCurrent: true}, kind: 'book', message: () => 'Rejected.', outcomeUnknown: () => false, notify});
  assert.equal(seen[0][0], 'Rejected. Your book draft is kept.');
  seen.length = 0;
  reportCreateFailure({locked: false, outcome: {error: 'e', keptCurrent: false}, kind: 'shelf', message: () => 'Rejected.', outcomeUnknown: () => false, notify});
  assert.match(seen[0][0], /^Rejected\. Your original shelf draft was discarded or replaced/);
});

// ---- the discard decision (bug: discard must not race a create, or lose B to a stale A) ----
// discardDraft performs the drop AND the re-acquire directly against
// `drafts`, not via injectable callbacks a mutation could redirect or drop:
// there is nothing left at the call site responsible for "fetch a fresh
// draft after a successful discard" -- the returned value already is one.

test('discardDraft refuses while the matching create is already in flight', async () => {
  const drafts = {}; const a = acquireDraft(drafts, 'k'); a.name = 'A';
  let confirmCalled = false;
  const fresh = await discardDraft({
    drafts, key: 'k', current: a,
    isBlocked: () => true,
    confirm: () => { confirmCalled = true; return Promise.resolve(true); },
  });
  assert.equal(fresh, null);
  assert.equal(confirmCalled, false, 'blocked discard never even asks');
  assert.equal(drafts['k'], a);
});

test('discardDraft re-checks after the confirmation dialog closes, in case a create started meanwhile', async () => {
  const drafts = {}; const a = acquireDraft(drafts, 'k'); a.name = 'A';
  let blocked = false;
  const confirmation = defer();
  const promise = discardDraft({
    drafts, key: 'k', current: a,
    isBlocked: () => blocked,
    confirm: () => confirmation.promise,
  });
  // The user is still looking at the confirm dialog when a create for this form starts.
  blocked = true;
  confirmation.resolve(true);
  const fresh = await promise;
  assert.equal(fresh, null);
  assert.equal(drafts['k'], a, 'a create that started while the dialog was open must still win');
});

test('discardDraft drops and returns a fresh, empty replacement for exactly the draft it was given, never another key', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  const other = acquireDraft(drafts, 'other'); other.name = 'untouched';
  const fresh = await discardDraft({drafts, key: 'k', current: a, isBlocked: () => false, confirm: () => Promise.resolve(true)});
  assert.notEqual(fresh, a);
  assert.deepEqual(fresh, {});
  assert.equal(drafts['k'], fresh, 'the returned draft must already be the live one, not a detached copy');
  assert.equal(drafts['other'], other, 'discarding one form must never touch another');

  const b = acquireDraft(drafts, 'k'); b.name = 'B';
  const declined = await discardDraft({drafts, key: 'k', current: b, isBlocked: () => false, confirm: () => Promise.resolve(false)});
  assert.equal(declined, null);
  assert.equal(drafts['k'], b);
});

test('discardDraft is identity-safe: discarding a stale reference never drops the current draft', async () => {
  const drafts = {};
  const a = acquireDraft(drafts, 'k'); a.name = 'A';
  dropDraft(drafts, 'k');
  const b = acquireDraft(drafts, 'k'); b.name = 'B';
  // `current` is `a`, a stale reference -- as if discard() had been asked
  // about a draft that something else already replaced.
  const fresh = await discardDraft({drafts, key: 'k', current: a, isBlocked: () => false, confirm: () => Promise.resolve(true)});
  assert.equal(fresh, null);
  assert.equal(drafts['k'], b);
});

// ---- form freezing: an identity switch, or this exact form's own create ----

test('identitySwitchInFlight freezes forms only for sign-out and login busy keys', () => {
  assert.equal(identitySwitchInFlight(true, 'signout'), true);
  assert.equal(identitySwitchInFlight(true, 'login'), true);
  assert.equal(identitySwitchInFlight(true, 'create-shelf'), false);
  assert.equal(identitySwitchInFlight(true, ''), false);
  assert.equal(identitySwitchInFlight(false, 'signout'), false);
});

test('formFrozen freezes for an identity switch or this form\'s own create, and nothing else', () => {
  assert.equal(formFrozen(true, 'signout', 'create-workspace'), true);
  assert.equal(formFrozen(true, 'login', 'create-shelf'), true);
  assert.equal(formFrozen(true, 'create-workspace', 'create-workspace'), true, 'this exact form\'s own create freezes it');
  assert.equal(formFrozen(true, 'create-shelf', 'create-book'), false, 'a different form\'s own create does not');
  assert.equal(formFrozen(false, '', 'create-workspace'), false);
  // Not busy, but busyKey happens to still equal formKey (a stale value):
  // must not freeze. Only an actually in-flight busy action (busy === true)
  // freezes anything.
  assert.equal(formFrozen(false, 'create-workspace', 'create-workspace'), false);
});

test('unsavedWorkSummary names kept drafts and an undismissed owner key, together or alone', () => {
  assert.equal(unsavedWorkSummary(['Create workspace draft'], true), 'Create workspace draft, the workspace owner key shown on screen');
  assert.equal(unsavedWorkSummary([], true), 'the workspace owner key shown on screen');
  assert.equal(unsavedWorkSummary(['A', 'B'], false), 'A, B');
  assert.equal(unsavedWorkSummary([], false), '');
});

test('hashSwitchBlocked refuses while busy, locked, or an unresolved pending/dirty edit blocks leaving', () => {
  assert.equal(hashSwitchBlocked(true, false, true), true, 'busy');
  assert.equal(hashSwitchBlocked(false, true, true), true, 'locked');
  assert.equal(hashSwitchBlocked(false, false, false), true, '!mayLeave()');
  assert.equal(hashSwitchBlocked(false, false, true), false, 'nothing blocking');
});

// ---- sign-out: confirm (only if there is something to lose), then verify, log out, reset, in that order ----

test('runSignOut signs out without asking when there is nothing to lose', async () => {
  let confirmCalled = false, resetCalled = false;
  const proceeded = await runSignOut({
    hasUnsavedWork: () => false,
    confirm: () => { confirmCalled = true; return Promise.resolve(true); },
    verify: async () => 'session',
    logout: async () => {},
    reset: () => { resetCalled = true; },
  });
  assert.equal(proceeded, true);
  assert.equal(confirmCalled, false);
  assert.equal(resetCalled, true);
});

test('runSignOut asks when there is unsaved work, and a decline skips verify/logout/reset entirely', async () => {
  let verifyCalled = false, logoutCalled = false, resetCalled = false;
  const proceeded = await runSignOut({
    hasUnsavedWork: () => true,
    confirm: () => Promise.resolve(false),
    verify: () => { verifyCalled = true; return Promise.resolve('session'); },
    logout: async () => { logoutCalled = true; },
    reset: () => { resetCalled = true; },
  });
  assert.equal(proceeded, false);
  assert.equal(verifyCalled, false);
  assert.equal(logoutCalled, false);
  assert.equal(resetCalled, false);
});

test('runSignOut verifies, logs out and resets in exactly that order', async () => {
  const order = [];
  const verifyDeferred = defer();
  const promise = runSignOut({
    hasUnsavedWork: () => true,
    confirm: async () => true,
    verify: () => { order.push('verify-start'); return verifyDeferred.promise; },
    logout: async session => { order.push('logout:' + session); },
    reset: () => order.push('reset'),
  });
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(order, ['verify-start'], 'logout/reset must not run before verify settles');
  verifyDeferred.resolve('SESSION');
  const proceeded = await promise;
  assert.equal(proceeded, true);
  assert.deepEqual(order, ['verify-start', 'logout:SESSION', 'reset']);
});

// ---- establishing a new identity: clear only after the request succeeds, before activating ----

test('runLogin clears drafts/owner-key only after the request succeeds, then activates', async () => {
  const order = [];
  await runLogin({
    request: async () => { order.push('request'); return 'SESSION'; },
    clearForNewIdentity: () => order.push('clear'),
    activate: async session => order.push('activate:' + session),
  });
  assert.deepEqual(order, ['request', 'clear', 'activate:SESSION']);
});

test('runLogin leaves drafts and the owner key untouched when the request is rejected', async () => {
  let cleared = false, activated = false;
  await assert.rejects(runLogin({
    request: () => Promise.reject(new Error('bad key')),
    clearForNewIdentity: () => { cleared = true; },
    activate: async () => { activated = true; },
  }), /bad key/);
  assert.equal(cleared, false);
  assert.equal(activated, false);
});

// ---- a hash-key sign-in over a live session -------------------------------
// The key is opaque, so whose identity it holds cannot be known before the
// request. Whenever there is unsaved work, runIdentitySwitch must ask BEFORE
// sending anything -- the server replaces the session cookie (and
// invalidates the old one) on every successful login, same identity or not,
// so a decline must truly send no request at all. identity is only compared
// AFTER a successful request, and never via the CSRF/session token (which
// always differs, even for the same key -- see model.js's identityKey()).

test('runIdentitySwitch asks before sending anything when there is unsaved work, and a decline sends no request', async () => {
  let requestCalled = false, adopted = false, cleared = false, activated = false;
  const outcome = await runIdentitySwitch({
    hasUnsavedWork: () => true,
    confirm: () => Promise.resolve(false),
    request: () => { requestCalled = true; return Promise.resolve('SESSION'); },
    sameIdentity: () => true,
    adoptSession: () => { adopted = true; },
    clearForNewIdentity: () => { cleared = true; },
    activate: async () => { activated = true; },
  });
  assert.equal(outcome, 'declined');
  assert.equal(requestCalled, false, 'a decline must never send the login request at all');
  assert.equal(adopted, false);
  assert.equal(cleared, false);
  assert.equal(activated, false);
});

test('runIdentitySwitch resolves the confirm before ever calling request (order, not just outcome)', async () => {
  const order = [];
  const confirmation = defer();
  const promise = runIdentitySwitch({
    hasUnsavedWork: () => true,
    confirm: () => { order.push('confirm-start'); return confirmation.promise; },
    request: async () => { order.push('request'); return 'SESSION'; },
    sameIdentity: () => true,
    adoptSession: () => order.push('adopt'),
    clearForNewIdentity: () => order.push('clear'),
    activate: async () => order.push('activate'),
  });
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(order, ['confirm-start'], 'request must not run before the confirm settles');
  confirmation.resolve(true);
  await promise;
  assert.deepEqual(order, ['confirm-start', 'request', 'adopt']);
});

test('runIdentitySwitch sends the request directly, without asking, when there is nothing to lose', async () => {
  let confirmCalled = false, requestCalled = false;
  const outcome = await runIdentitySwitch({
    hasUnsavedWork: () => false,
    confirm: () => { confirmCalled = true; return Promise.resolve(true); },
    request: async () => { requestCalled = true; return 'SESSION'; },
    sameIdentity: () => false,
    adoptSession: () => {}, clearForNewIdentity: () => {}, activate: async () => {},
  });
  assert.equal(outcome, 'switched');
  assert.equal(confirmCalled, false);
  assert.equal(requestCalled, true);
});

test('runIdentitySwitch adopts the new session/CSRF for the same identity, keeping every draft and the owner key', async () => {
  let cleared = false, activated = false, adoptedWith = null;
  const outcome = await runIdentitySwitch({
    hasUnsavedWork: () => true,
    confirm: () => Promise.resolve(true),
    request: async () => 'NEW_SESSION_SAME_IDENTITY',
    sameIdentity: session => session === 'NEW_SESSION_SAME_IDENTITY',
    adoptSession: session => { adoptedWith = session; },
    clearForNewIdentity: () => { cleared = true; },
    activate: async () => { activated = true; },
  });
  assert.equal(outcome, 'same');
  assert.equal(adoptedWith, 'NEW_SESSION_SAME_IDENTITY');
  assert.equal(cleared, false, 'the same identity must never clear drafts or the owner key');
  assert.equal(activated, false, 'adopting in place must not run the full activate (no workspace/view reset)');
});

test('runIdentitySwitch clears then activates for a different identity, in that order', async () => {
  const order = [];
  const outcome = await runIdentitySwitch({
    hasUnsavedWork: () => true,
    confirm: () => Promise.resolve(true),
    request: async () => 'OTHER_SESSION',
    sameIdentity: () => false,
    adoptSession: () => order.push('adopt'),
    clearForNewIdentity: () => order.push('clear'),
    activate: async session => order.push('activate:' + session),
  });
  assert.equal(outcome, 'switched');
  assert.deepEqual(order, ['clear', 'activate:OTHER_SESSION']);
});

test('runIdentitySwitch leaves drafts, the owner key and the current session untouched when a confirmed request is rejected', async () => {
  let adopted = false, cleared = false, activated = false;
  await assert.rejects(runIdentitySwitch({
    hasUnsavedWork: () => true,
    confirm: () => Promise.resolve(true),
    request: () => Promise.reject(new Error('bad key')),
    sameIdentity: () => false,
    adoptSession: () => { adopted = true; }, clearForNewIdentity: () => { cleared = true; }, activate: async () => { activated = true; },
  }), /bad key/);
  assert.equal(adopted, false);
  assert.equal(cleared, false);
  assert.equal(activated, false);
});

// ---- workspace creation's one-time owner key: block a second create, never drop its own draft unconditionally ----

test('createWorkspaceFlow blocks while the previous owner key is undismissed, without even trying the write', async () => {
  let requestCalled = false;
  const outcome = await createWorkspaceFlow({
    hasUndismissedOwnerKey: () => true,
    drafts: {}, key: 'k', submitted: {},
    request: () => { requestCalled = true; return Promise.resolve({}); },
    confirmedText: () => '',
  });
  assert.deepEqual(outcome, {ok: false, blocked: true});
  assert.equal(requestCalled, false);
});

test('createWorkspaceFlow behaves exactly like submitDraftCreate once unblocked', async () => {
  const drafts = {}; const a = acquireDraft(drafts, 'k'); a.name = 'A';
  const ok = await createWorkspaceFlow({
    hasUndismissedOwnerKey: () => false,
    drafts, key: 'k', submitted: a,
    request: async () => ({workspace_id: 'ws-1'}),
    confirmedText: () => '',
  });
  assert.equal(ok.ok, true);
  assert.equal(ok.blocked, false);
  assert.equal('k' in drafts, false);

  const b = acquireDraft(drafts, 'k'); b.name = 'B';
  const failed = await createWorkspaceFlow({
    hasUndismissedOwnerKey: () => false,
    drafts, key: 'k', submitted: b,
    request: async () => { throw new Error('503'); },
    confirmedText: () => '',
  });
  assert.equal(failed.ok, false);
  assert.equal(failed.blocked, false);
  assert.equal(failed.keptCurrent, true);
});

// ---- one hasUnsavedWork(state), shared by sign-out, the hash switch, beforeunload and LockedView ----

test('hasUnsavedWork is true for a kept draft, an undismissed owner key, either, neither or both', () => {
  assert.equal(hasUnsavedWork({drafts: {}, createdWorkspace: null}), false);
  assert.equal(hasUnsavedWork({drafts: {k: {name: ''}}, createdWorkspace: null}), false, 'a blank draft is not unsaved work');
  assert.equal(hasUnsavedWork({drafts: {k: {name: 'Research'}}, createdWorkspace: null}), true);
  assert.equal(hasUnsavedWork({drafts: {}, createdWorkspace: {workspace_id: 'w', owner_key: 'k'}}), true);
  assert.equal(hasUnsavedWork({drafts: {k: {name: 'Research'}}, createdWorkspace: {workspace_id: 'w', owner_key: 'k'}}), true);
});

// ---- what signing out, or activating a new identity, actually clears ----

test('clearedSessionState clears the session, every draft, the owner key, and returns to overview', () => {
  const cleared = clearedSessionState();
  assert.deepEqual(cleared, {
    session: null, workspace: null, inspection: null, workspaces: [], workspacesNext: null, workspacesAfter: null,
    dirty: false, pending: null, draft: '', drafts: {}, locked: false, lockMessage: '', createdWorkspace: null, view: 'overview',
  });
});

test('clearedSessionState returns fresh objects each time (no shared mutable state across resets)', () => {
  const first = clearedSessionState();
  first.drafts.leaked = {name: 'should not appear in the next reset'};
  first.workspaces.push('should not appear either');
  const second = clearedSessionState();
  assert.deepEqual(second.drafts, {});
  assert.deepEqual(second.workspaces, []);
});

// ---- the focus-handoff effect's dependency key must change when `activating` does ----

test('focusHandoffKey changes with activating, independently of session, locked, workspace and view', () => {
  const base = focusHandoffKey({}, false, true, 'ws', 'overview');
  assert.notEqual(focusHandoffKey({}, false, false, 'ws', 'overview'), base);
  assert.notEqual(focusHandoffKey(null, false, true, 'ws', 'overview'), base);
  assert.notEqual(focusHandoffKey({}, true, true, 'ws', 'overview'), base);
  assert.notEqual(focusHandoffKey({}, false, true, 'other', 'overview'), base);
  assert.notEqual(focusHandoffKey({}, false, true, 'ws', 'context'), base);
  assert.equal(focusHandoffKey({}, false, true, 'ws', 'overview'), base);
  assert.equal(focusHandoffKey({}, false, true, undefined, 'overview'), focusHandoffKey({}, false, true, '', 'overview'));
});


// These tests drive the actual request/verify/adoption/discard implementations.
// No test calls a create's confirmed callback itself. The mock models the Rust
// login handler: success rotates cookie + CSRF and invalidates the previous
// token; rejection changes neither. A held request retains its dispatch cookie.
import {sessionRequest, verifySession, adoptSession, creationRecovery} from '../crates/server/web/src/lib/sessionRequests.js';
import {discardAndSignIn} from '../crates/server/web/src/lib/lockedSignIn.js';

function browserSessionServer() {
  let serial = 1, cookie = 'token-1';
  const sessions = new Map([[cookie, {...session(), csrf_token: 'csrf-1'}]]);
  const holds = new Map();
  const requests = [];
  const created = {workspace_id: 'created-workspace', owner_key: 'one-time-test-owner'};
  const fail = (status, message) => Object.assign(new Error(message), {status});
  function hold(path) {
    const started = defer(), response = defer();
    const gate = {started: started.promise, release: null};
    const queue = holds.get(path) ?? [];
    queue.push({started, response, gate}); holds.set(path, queue);
    return gate;
  }
  function wire(path, body, csrf) {
    const token = cookie;
    requests.push({path, token, csrf});
    function reply() {
      if (path.endsWith('/login')) {
        if (csrf !== 'login' || JSON.parse(body).key !== 'accepted-test-key') throw fail(401, 'Invalid key');
        const next = {...session(), csrf_token: `csrf-${++serial}`};
        sessions.delete(cookie); cookie = `token-${serial}`; sessions.set(cookie, next);
        return next;
      }
      const current = sessions.get(token);
      if (!current) throw fail(401, 'Expired browser session');
      if (body !== undefined && csrf !== current.csrf_token) throw fail(403, 'Invalid CSRF');
      if (path === '/denied') throw fail(403, 'Permission revoked');
      if (path.endsWith('/create')) return created;
      return current;
    }
    const held = holds.get(path)?.shift();
    if (held) {
      held.gate.release = () => { try { held.response.resolve(reply()); } catch (error) { held.response.reject(error); } };
      held.started.resolve({token, csrf});
      return held.response.promise;
    }
    try { return Promise.resolve(reply()); } catch (error) { return Promise.reject(error); }
  }
  return {wire, hold, created, requests, current: () => sessions.get(cookie), cookie: () => cookie};
}

function requestHarness() {
  const server = browserSessionServer();
  const state = {...clearedSessionState(), session: server.current(), generation: 0, inflight: 0};
  const deps = {wire: server.wire, lock: message => { state.locked = true; state.lockMessage = message; state.generation++; }};
  state.recoveryText = () => recoveryText(state.pending, state.draft, state.drafts);
  const request = (path, body, confirmed) => sessionRequest(state, deps, path, body, confirmed);
  function create() {
    const submitted = acquireDraft(state.drafts, WORKSPACE_DRAFT_KEY); submitted.name = 'Created';
    return createWorkspaceFlow({
      hasUndismissedOwnerKey: () => state.createdWorkspace !== null,
      drafts: state.drafts, key: WORKSPACE_DRAFT_KEY, submitted,
      request: confirmed => request('/api/v1/admin/workspaces/create', {name: submitted.name}, confirmed),
      confirmedText: data => { state.createdWorkspace = data; return creationRecovery('/api/v1/admin/workspaces/create', data); },
    });
  }
  const login = () => runIdentitySwitch({
    hasUnsavedWork: () => hasUnsavedWork(state), confirm: async () => true,
    request: () => server.wire('/api/v1/admin/session/login', JSON.stringify({key: 'accepted-test-key'}), 'login'),
    sameIdentity: next => identityKey(next) === identityKey(state.session),
    adoptSession: next => adoptSession(state, next),
    clearForNewIdentity: () => { throw new Error('same identity must preserve local work'); },
    activate: async () => { throw new Error('same identity must adopt in place'); },
  });
  return {server, state, deps, request, create, login};
}

test('request ordering: a concurrent lock after successful post-create verification keeps the key and drops the submitted draft', async () => {
  const h = requestHarness();
  const write = h.server.hold('/api/v1/admin/workspaces/create');
  const creating = h.create(); await write.started;
  const postcheck = h.server.hold('/api/v1/admin/session');
  write.release(); await postcheck.started;
  const denied = h.server.hold('/denied');
  const other = assert.rejects(h.request('/denied'), /Permission revoked/); await denied.started;
  // Resolving these in order lets verify succeed, then locks from the other
  // request before the create's continuation resumes (FIFO promise jobs).
  postcheck.release(); denied.release();
  await other;
  const outcome = await creating;
  assert.equal(outcome.ok, false);
  assert.equal(h.state.locked, true);
  assert.equal(h.state.createdWorkspace, h.server.created);
  assert.equal(WORKSPACE_DRAFT_KEY in h.state.drafts, false);
  assert.equal(h.state.inflight, 0);
});

test('discard ordering: an owner key arriving during the open sign-in confirmation requires a fresh decision', async () => {
  const h = requestHarness();
  const write = h.server.hold('/api/v1/admin/workspaces/create');
  const creating = h.create(); await write.started;
  h.deps.lock('Session ended');
  const choice = defer(); let dialog;
  const reset = () => Object.assign(h.state, clearedSessionState());
  const discarding = discardAndSignIn(h.state, options => { dialog = options; return choice.promise; }, reset);
  assert.doesNotMatch(dialog.body, /owner key/);
  write.release(); await creating;
  assert.equal(h.state.createdWorkspace, h.server.created);
  choice.resolve(true); await discarding;
  assert.equal(h.state.createdWorkspace, h.server.created, 'old approval never covered the new owner key');
  assert.equal(h.state.locked, true);
  await discardAndSignIn(h.state, async options => { assert.match(options.body, /owner key/); return true; }, reset);
  assert.equal(h.state.createdWorkspace, null, 'a fresh explicit approval may discard it');
  assert.equal(h.state.session, null);
});

test('adoption ordering: a same-identity login completing during a lock unlocks the rotated session and preserves local work', async () => {
  const h = requestHarness();
  acquireDraft(h.state.drafts, 'k').name = 'Kept'; h.state.createdWorkspace = h.server.created;
  const original = h.state.session, drafts = h.state.drafts, cookie = h.server.cookie();
  const login = h.server.hold('/api/v1/admin/session/login');
  const switching = h.login(); await login.started;
  h.deps.lock('Old session expired'); login.release();
  assert.equal(await switching, 'same');
  assert.notEqual(h.server.cookie(), cookie); assert.notEqual(h.state.session.csrf_token, original.csrf_token);
  assert.equal(h.state.session, h.server.current());
  assert.equal(h.state.locked, false); assert.equal(h.state.lockMessage, '');
  assert.equal(h.state.drafts, drafts); assert.equal(h.state.createdWorkspace, h.server.created);
});

test('adoption ordering: a late write 401 under the invalidated token cannot lock the adopted session', async () => {
  const h = requestHarness();
  const cookie = h.server.cookie(), original = h.server.current();
  await assert.rejects(h.server.wire('/api/v1/admin/session/login', JSON.stringify({key: 'rejected-test-key'}), 'login'), /Invalid key/);
  assert.equal(h.server.cookie(), cookie); assert.equal(h.server.current(), original, 'failed login never replaces the session');
  const write = h.server.hold('/write');
  const writing = assert.rejects(h.request('/write', {}), /Expired browser session/); await write.started;
  await h.login(); const adopted = h.state.session, generation = h.state.generation;
  write.release(); await writing;
  assert.equal(h.state.session, adopted); assert.equal(h.state.locked, false);
  assert.equal(h.state.generation, generation); assert.equal(h.state.lockMessage, '');
});

test('adoption ordering: a late preflight 401 under the invalidated token cannot lock the adopted session', async () => {
  const h = requestHarness();
  const preflight = h.server.hold('/api/v1/admin/session');
  const verifying = assert.rejects(verifySession(h.state, h.deps), /Expired browser session/); await preflight.started;
  await h.login(); const generation = h.state.generation;
  preflight.release(); await verifying;
  assert.equal(h.state.locked, false); assert.equal(h.state.generation, generation);
});

test('adoption ordering: a login between preflight success and request continuation prevents dispatch under the replacement session', async () => {
  const h = requestHarness();
  const preflight = h.server.hold('/api/v1/admin/session');
  const writing = h.request('/write', {}); const rejected = assert.rejects(writing, /session changed|view changed/i);
  await preflight.started;
  const login = h.server.hold('/api/v1/admin/session/login');
  const switching = h.login(); await login.started;
  preflight.release(); login.release();
  await switching; await rejected;
  assert.equal(h.server.requests.some(request => request.path === '/write'), false, 'preflight approval cannot authorize a write under the adopted session');
  assert.equal(h.state.locked, false); assert.equal(h.state.inflight, 0);
});

test('adoption ordering: a superseded post-create verification cannot clear pending work or lock the adopted session', async () => {
  const h = requestHarness();
  const write = h.server.hold('/api/v1/admin/workspaces/create');
  const creating = h.create(); await write.started;
  const postcheck = h.server.hold('/api/v1/admin/session');
  write.release(); await postcheck.started;
  await h.login(); const pending = {body: 'new-session-work'}; h.state.pending = pending;
  const generation = h.state.generation;
  postcheck.release(); await creating;
  assert.equal(h.state.locked, false); assert.equal(h.state.generation, generation);
  assert.equal(h.state.pending, pending); assert.equal(h.state.inflight, 0);
});

test('create recovery: a failed post-write verification describes the owner key actually displayed', async () => {
  const h = requestHarness();
  const write = h.server.hold('/api/v1/admin/workspaces/create');
  const creating = h.create(); await write.started;
  h.deps.lock('Session ended'); write.release(); await creating;
  assert.equal(h.state.createdWorkspace, h.server.created);
  assert.match(h.state.lockMessage, /owner key shown below/);
  assert.doesNotMatch(h.state.lockMessage, /no longer displayed|reset-owner-key/);
  assert.equal(WORKSPACE_DRAFT_KEY in h.state.drafts, false);
});

import './workspace_panel_wiring.mjs';
