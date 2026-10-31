// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {takeSetupCode, setupRequest, setupKeys, validSetupName, SetupError} from '../crates/server/web/src/lib/setup.js';
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
