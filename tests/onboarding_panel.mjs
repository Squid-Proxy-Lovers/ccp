// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {takeSetupCode, setupRequest, setupKeys, validSetupName, SetupError} from '../crates/server/web/src/lib/setup.js';
import {isStrictUuid} from '../crates/server/web/src/lib/model.js';
import {parseEnrollmentLink} from '../crates/server/web/src/lib/enrollment_link.js';
import {LinkAttempt} from '../crates/server/web/src/lib/link_issuance.js';
import {readFile} from 'node:fs/promises';
const instance = '11111111-1111-4111-8111-111111111111';
const code = 'ccp_setup_' + 'a'.repeat(64);

test('setup fragments are removed before parsing and never routed through admin login', () => {
  const calls=[]; const history={replaceState:(...args)=>calls.push(args)};
  assert.equal(takeSetupCode({hash:'#code='+code},history),code);
  assert.deepEqual(calls,[[null,'','/admin/setup']]);
  for (const hash of ['#code=bad','#code='+code+'&x=1','#code='+code+'&code='+code,'#key='+code,'#code=%zz']) {
    const events=[];assert.throws(()=>takeSetupCode({hash},{replaceState:(...args)=>events.push(args)}),error=>!error.message.includes(code));
    assert.deepEqual(events,[[null,'','/admin/setup']]);
  }
  assert.equal(takeSetupCode({hash:''},{replaceState:()=>assert.fail('empty fragment')}),'');
});
test('claim and verify use only exact protected POSTs and do not retry or echo secrets', async () => {
  const original=globalThis.fetch;let calls=0;
  try {
    globalThis.fetch=async(path,options)=>{
      calls+=1;assert.equal(path,'/api/v1/setup/claim');assert.equal(options.method,'POST');
      assert.equal(options.credentials,'same-origin');assert.equal(options.cache,'no-store');assert.equal(options.redirect,'error');
      assert.equal(options.headers['X-CCP-CSRF'],'setup');assert.equal(JSON.parse(options.body).code,code);
      throw new Error(code);
    };
    await assert.rejects(setupRequest('claim',{code,operator:'owner',workspace:'first'}),error=>error instanceof SetupError && error.uncertain && !error.message.includes(code));
    assert.equal(calls,1);
    globalThis.fetch=async()=>({ok:false,status:503,json:async()=>({error:{code:'PRIVATE_DATABASE_ERROR'}})});
    await assert.rejects(setupRequest('claim',{code}),error=>error.status===503 && error.uncertain && !error.message.includes('PRIVATE_DATABASE_ERROR'));
    for (const status of [400,401,403,409,410,413,429]) {
      globalThis.fetch=async()=>({ok:false,status,json:async()=>({error:{code:'PRIVATE_DATABASE_ERROR'}})});
      await assert.rejects(setupRequest('claim',{code}),error=>error.status===status && !error.uncertain && !error.message.includes('PRIVATE_DATABASE_ERROR'));
    }
    globalThis.fetch=async()=>({ok:true,status:200,json:async()=>({api_version:1,data:{verified:true,instance_id:instance}})});
    assert.deepEqual(await setupRequest('verify',{code}),{verified:true,instance_id:instance});
    await assert.rejects(setupRequest('reset',{code}),/Invalid setup action/);
  } finally {globalThis.fetch=original;}
});
test('invalid JSON and unknown success envelopes keep claim outcomes uncertain', async () => {
  const original=globalThis.fetch;
  try {
    globalThis.fetch=async()=>({ok:true,status:200,json:async()=>{throw new Error(code);}});
    await assert.rejects(setupRequest('claim',{code}),error=>error.uncertain && !error.message.includes(code));
    globalThis.fetch=async()=>({ok:true,status:200,json:async()=>({api_version:2,data:{}})});
    await assert.rejects(setupRequest('claim',{code}),error=>error.uncertain);
  } finally {globalThis.fetch=original;}
});
test('setup names use UTF8 byte limits and malformed key receipts cannot open the panel', () => {
  assert.equal(validSetupName('owner'),true);assert.equal(validSetupName('é'.repeat(128)),true);
  for (const name of ['', '   ','é'.repeat(129),'line\nbreak','a'.repeat(257)]) assert.equal(validSetupName(name),false);
  const receipt={instance_id:instance,workspace_id:instance,principal_id:instance,instance_admin_key:{secret:'ccp_admin_'+'a'.repeat(64)},workspace_owner_key:{secret:'ccp_admin_'+'b'.repeat(64)}};
  assert.equal(setupKeys(receipt),receipt);
  for (const value of [null,{}, {...receipt,workspace_id:'not-an-id'}, {...receipt,workspace_owner_key:{secret:'foreign'}}]) assert.throws(()=>setupKeys(value),error=>error.uncertain);
});

test('setup rejects nil identifiers in verification and every claim receipt field', async () => {
  const nil = '00000000-0000-0000-0000-000000000000';
  const keys = {instance_id: instance, workspace_id: instance, principal_id: instance,
    instance_admin_key: {secret: 'ccp_admin_'+'a'.repeat(64)}, workspace_owner_key: {secret: 'ccp_admin_'+'b'.repeat(64)}};
  for (const field of ['instance_id', 'workspace_id', 'principal_id']) {
    assert.throws(() => setupKeys({...keys, [field]: nil}), error => error instanceof SetupError && error.uncertain);
  }
  assert.throws(() => setupKeys({...keys, instance_id: nil, workspace_id: nil, principal_id: nil}), SetupError);
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async () => ({ok: true, status: 200, json: async () => ({api_version: 1, data: {verified: true, instance_id: nil}})});
    await assert.rejects(setupRequest('verify', {}), error => error instanceof SetupError && !error.uncertain);
  } finally { globalThis.fetch = original; }
});

// Oracle tests for the refactor, deliberately retaining the old predicates.
// This finite corpus covers every UUID position, valid generated identifiers,
// malformed strings and JS type coercion boundaries without node_modules.
function uuidInputs() {
  const format = hex => `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
  const seed = 'abcdef01-2345-6789-abcd-ef0123456789';
  const strings = new Set(['', '00000000-0000-0000-0000-000000000000', seed, seed.toUpperCase(),
    `{${seed}}`, `urn:uuid:${seed}`, seed.replaceAll('-', ''), 'é'.repeat(36), '０'.repeat(36)]);
  const characters = ['0', 'f', 'A', 'F', 'g', 'G', '-', '_', ' ', '\t', '\n', '\r', '\0', '\u200b', '\ufeff', '\ud800', 'é', '０'];
  for (let index = 0; index < seed.length; index++) {
    for (const char of characters) strings.add(seed.slice(0,index) + char + seed.slice(index+1));
    strings.add(seed.slice(0,index) + seed.slice(index+1));
  }
  for (const char of characters) { strings.add(char + seed); strings.add(seed + char); }
  for (let index = 0; index < 512; index++) {
    strings.add(format(index.toString(16).padStart(32, '0')));
    strings.add(format(index.toString(16).padEnd(32, 'f')));
  }
  return [...strings, null, undefined, false, true, 0, 1, NaN, Infinity, 1n, Symbol('uuid'),
    [], [seed], {}, new String(seed), {toString: () => seed}];
}

test('shared UUID checks preserve non-nil acceptance and reject nil setup identifiers across varied inputs', async t => {
  const oldSetup = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value);
  const oldLink = value => oldSetup(value) && value.replaceAll('-', '') !== '0'.repeat(32);
  const accepts = fn => { try { fn(); return true; } catch { return false; } };
  const vectors = JSON.parse(await readFile(new URL('../crates/protocol/testdata/enrollment_links.json', import.meta.url)));
  const uri = vectors.cases[0].uri.replaceAll('{invitation}', 'ccp_join_' + vectors.invitation_suffix);
  const link = parseEnrollmentLink(uri);
  const session = {instance_id: link.instance_id, csrf_token: 'proof', identity: {key_id: 'key', principal_id: 'owner',
    scope: {kind: 'workspace', workspace_id: link.workspace_id, role: 'owner'}}};
  const attempt = new LinkAttempt(session, {id: link.workspace_id, archived: false}, 'read', {});
  const receipt = {enrollment_link: uri, workspace_id: link.workspace_id, workspace_name: link.workspace_name,
    expires_at: Math.floor(Date.now()/1000)+1800, max_uses: 1};
  const keys = {instance_id: instance, workspace_id: instance, principal_id: instance,
    instance_admin_key: {secret: 'ccp_admin_'+'a'.repeat(64)}, workspace_owner_key: {secret: 'ccp_admin_'+'b'.repeat(64)}};
  const original = globalThis.fetch;
  const inputs = uuidInputs(); let setupAccepted = 0, linkAccepted = 0;
  try {
    for (const value of inputs) {
      const expectedSetup = oldSetup(value) && value !== '00000000-0000-0000-0000-000000000000';
      const label = typeof value === 'string' ? JSON.stringify(value) : typeof value;
      assert.equal(isStrictUuid(value), oldLink(value), `strict predicate: ${label}`);
      for (const field of ['instance_id', 'workspace_id', 'principal_id']) {
        assert.equal(accepts(() => setupKeys({...keys, [field]: value})), expectedSetup, `setup ${field}: ${label}`);
      }
      globalThis.fetch = async () => ({ok: true, status: 200, json: async () => ({api_version: 1, data: {verified: true, instance_id: value}})});
      const verified = await setupRequest('verify', {}).then(() => true, () => false);
      assert.equal(verified, expectedSetup, `setup verify: ${label}`);
      assert.equal(accepts(() => attempt.receipt({...receipt, invitation_id: value})), oldLink(value), `issuance: ${label}`);
      if (typeof value === 'string') for (const field of ['i', 'w']) {
        const url = new URL(uri), params = new URLSearchParams(url.hash.slice(1));
        params.set(field, value); url.hash = params.toString();
        assert.equal(accepts(() => parseEnrollmentLink(url.href)), oldLink(value), `link ${field}: ${label}`);
      }
      setupAccepted += Number(expectedSetup); linkAccepted += Number(oldLink(value));
    }
  } finally { globalThis.fetch = original; }
  assert.ok(inputs.length > 1600); assert.ok(linkAccepted > 1000);
  assert.equal(setupAccepted, linkAccepted, 'all three validators now reject nil UUIDs');
  t.diagnostic(`${inputs.length} inputs: ${setupAccepted} accepted by setup, ${linkAccepted} accepted by links and issuance`);
});
