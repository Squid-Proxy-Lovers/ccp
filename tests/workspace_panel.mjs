// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source = await readFile(new URL('../crates/server/src/workspace_http/panel/model.js', import.meta.url), 'utf8');
const {WorkspaceDeletionIntent, canDeleteWorkspace, deletionPreview, deletionLookup, deletionStatusRequest, deletionReceipt, deletionTracking, StatusClearIntent, reportedWorkStatus, reportedWorkPage, ContentAdminIntent, shelfState, archivedShelfPage, ClientAccessAttempt, clientRecoveryScope, recoveryStatus, recoveryPage, EnrollmentAttempt, enrollmentStatus, initialBounds, AdmissionAttempt, canManageInvitations, invitationDeadline, invitationStatus, invitationPage, TransferAttempt, canTransferOwnership, SecurityAttempt, canManageTeam, keyExpiry, Intent, GuidanceIntent, LifecycleAttempt, RetentionIntent, canChangeLifecycle, retentionSeconds, retentionPolicy, workspaceSnapshot, canPublish, guidanceBundle, signature, canWrite, exactRevision, takeFragment, wire} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

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
