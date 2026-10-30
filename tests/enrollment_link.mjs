// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {parseEnrollmentLink, enrollmentUri, enrollmentCommand, takeJoinLink, watchJoinLinks, EnrollmentLinkError} from '../crates/server/web/src/lib/enrollment_link.js';
const fixture = JSON.parse(readFileSync(new URL('../crates/protocol/testdata/enrollment_links.json', import.meta.url)));
const invitation = 'ccp_join_' + fixture.invitation_suffix;

test('browser and Rust use the same valid and hostile link vectors', () => {
  for (const item of fixture.cases) {
    const uri = item.uri.replaceAll('{invitation}', invitation);
    if (!item.valid) {
      assert.throws(() => parseEnrollmentLink(uri), error => error instanceof EnrollmentLinkError && error.kind === item.error && !error.message.includes(invitation), item.name);
      continue;
    }
    const link = parseEnrollmentLink(uri);
    assert.equal(link.https_origin, item.https_origin, item.name);
    assert.equal(link.mtls_origin, item.mtls_origin, item.name);
    assert.equal(link.workspace_name, item.workspace_name, item.name);
    assert.equal(link.invitation, invitation);
    assert.equal(link.instance_id, item.i);
    assert.equal(link.workspace_id, item.w);
    assert.equal(link.ca_pin, item.ca);
    const canonical = enrollmentUri(link);
    assert.equal(canonical, item.canonical.replaceAll('{invitation}', invitation), item.name);
    assert.deepEqual(parseEnrollmentLink(canonical), link);
    assert.equal(enrollmentUri(parseEnrollmentLink(canonical)), canonical);
    assert.equal(canonical.split('#')[0], item.https_origin + 'join');
    assert.equal(canonical.split('#')[0].includes(invitation), false);
    assert.equal(/[ '"\r\n]/.test(canonical), false);
    assert.equal(enrollmentCommand(link), "ccp enroll '" + canonical + "'");
  }
});
test('join clears every fragment before validation, even invalid ones', () => {
  const uri = fixture.cases[0].uri.replaceAll('{invitation}', invitation);
  const url = new URL(uri), calls = [];
  const link = takeJoinLink(url, {replaceState: (...args) => calls.push(args)});
  assert.equal(link.invitation, invitation);
  assert.deepEqual(calls, [[null, '', '/join']]);
  const rejected = [];
  assert.throws(() => takeJoinLink({origin: url.origin, hash: '#v=unknown&t=' + invitation}, {replaceState: (...args) => rejected.push(args)}), EnrollmentLinkError);
  assert.deepEqual(rejected, [[null, '', '/join']]);
  assert.equal(takeJoinLink({origin: url.origin, hash: ''}, {replaceState: () => assert.fail()}), null);
});
test('link operations cannot submit a secret, persist state, or silently replace malformed Unicode', () => {
  const original = globalThis.fetch; let requests = 0;
  globalThis.fetch = () => { requests++; throw new Error('network forbidden'); };
  try {
    const uri = fixture.cases[0].uri.replaceAll('{invitation}', invitation);
    const link = parseEnrollmentLink(uri);
    enrollmentCommand(link); enrollmentUri(link);
    assert.equal(Object.isFrozen(link), true);
    assert.equal(Object.keys(link).includes('invitation'), false);
    assert.equal(JSON.stringify(link).includes(invitation), false);
    assert.equal({...link}.invitation, undefined);
    for (const input of [null, {}, 0, uri + '\ud800', uri.replace('Team+context', '\udfff')]) {
      assert.throws(() => parseEnrollmentLink(input), error => error instanceof EnrollmentLinkError && !error.message.includes(invitation));
    }
    assert.equal(requests, 0);
  } finally { globalThis.fetch = original; }
});

test('canonical links preserve generated names, identifiers, pins and endpoint choices', () => {
  const base = parseEnrollmentLink(fixture.cases[0].uri.replaceAll('{invitation}', invitation));
  const names = ["Bob's", 'name+/&=%#~*', '名 🦑', '\ufeffname', 'a\\b', 'é'.repeat(128)];
  for (let index = 1; index <= 256; index++) {
    const link = {...base, invitation: base.invitation,
      workspace_id: index.toString(16).padStart(8, '0') + '-1111-4111-8111-000000000001',
      mtls_origin: 'https://client-' + index + '.example:' + (8000 + index) + '/',
      workspace_name: names[index % names.length],
    };
    const uri = enrollmentUri(link);
    const parsed = parseEnrollmentLink(uri);
    const {invitation: secret, ...metadata} = link;
    assert.deepEqual({...parsed}, metadata);
    assert.equal(parsed.invitation, secret);
    assert.equal(enrollmentUri(parseEnrollmentLink(uri)), uri);
    assert.equal(/[ '"\r\n]/.test(uri), false);
    for (const field of ['v', 'i', 'm', 'ca', 't', 'w', 'n']) {
      assert.throws(() => parseEnrollmentLink(uri + '&' + field + '=other'), EnrollmentLinkError);
    }
  }
});
test('all 64 possible final base64url symbols obey the SHA256 pad-bit contract', () => {
  const uri = fixture.cases[0].uri.replaceAll('{invitation}', invitation);
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  for (const [index, symbol] of [...alphabet].entries()) {
    const input = uri.replace('ca=sha256%3A' + 'A'.repeat(43), 'ca=sha256%3A' + 'A'.repeat(42) + symbol);
    if (index % 4 === 0) assert.doesNotThrow(() => parseEnrollmentLink(input));
    else assert.throws(() => parseEnrollmentLink(input), EnrollmentLinkError);
  }
});

const {LinkAttempt, linkSettings, linkDateIsLong, displayLinkName} = await import('../crates/server/web/src/lib/link_issuance.js');
const baseLink = parseEnrollmentLink(fixture.cases[0].uri.replaceAll('{invitation}', invitation));
const owner = () => ({instance_id: baseLink.instance_id, csrf_token: 'original-proof', identity: {
  key_id: 'owner-key', principal_id: 'owner', scope: {kind: 'workspace', workspace_id: baseLink.workspace_id, role: 'owner'},
}});
const workspace = {id: baseLink.workspace_id, name: baseLink.workspace_name, archived: false};
const unused = () => ({workspace_id: workspace.id, phase: 'unused', closed_at: null,
  has_admission_history: false, workspace_archived: false, can_open: true, invitation: null});
const now = new Date(2026, 9, 29, 12, 0).getTime();
const localDate = seconds => {
  const d = new Date(now + seconds * 1000);
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}T${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}:${String(d.getSeconds()).padStart(2,'0')}`;
};
const receipt = (extra = {}) => ({invitation_id: '11111111-1111-4111-8111-111111111111',
  workspace_id: workspace.id, workspace_name: baseLink.workspace_name, enrollment_link: enrollmentUri(baseLink),
  expires_at: Math.floor(now/1000)+1800, max_uses: 1, ...extra});

test('link settings retain finite defaults, custom dates, explicit never and seven-day boundary', () => {
  assert.deepEqual(linkSettings({}, false, now), {admission: {kind:'individual'}, lifetime:{kind:'short'},https_origin:null,mtls_origin:null});
  for (const seconds of [1, 86400, 7*86400-600]) {
    const settings = linkSettings({mode:'at',date:localDate(seconds)}, false, now);
    assert.equal(settings.lifetime.expires_at, Math.floor(now/1000)+seconds);
    assert.equal(settings.lifetime.confirm_long_lived, false);
  }
  assert.throws(() => linkSettings({mode:'at',date:localDate(7*86400+1)},false,now),/seven days/);
  assert.equal(linkDateIsLong(localDate(7*86400),now),true);
  assert.equal(linkDateIsLong(localDate(7*86400+1),now),true);
  assert.equal(linkSettings({mode:'at',date:localDate(90*86400),longLived:true},false,now).lifetime.confirm_long_lived,true);
  assert.throws(() => linkSettings({mode:'never'},false,now),/no expiry/);
  assert.deepEqual(linkSettings({mode:'never',noExpiry:true},false,now).lifetime,{kind:'never',confirm_no_expiry:true});
  for (const date of ['', 'invalid', localDate(0), localDate(-1)]) assert.throws(() => linkSettings({mode:'at',date},false,now),/future expiry/);
});
test('long-lived initial groups require positive caps and advanced origins share strict link rules', () => {
  assert.equal(linkSettings({},true,now).admission.max_clients,null);
  for (const fields of [{mode:'never',noExpiry:true},{mode:'at',date:localDate(90*86400),longLived:true}]) {
    for (const capacity of ['', '0', '-1', '1.5', '4294967296']) assert.throws(()=>linkSettings({...fields,capacity},true,now),/cap/);
    assert.equal(linkSettings({...fields,capacity:'2'},true,now).admission.max_clients,2);
  }
  const settings = linkSettings({https:'https://public.example:9443',mtls:'https://clients.example:9444'},false,now);
  assert.equal(settings.https_origin,'https://public.example:9443/');
  assert.equal(settings.mtls_origin,'https://clients.example:9444/');
  for (const value of ['http://bad.example','https://user@host','https://host/path','https://host?secret=1','https://[::ffff:127.0.0.1]']) {
    assert.throws(()=>linkSettings({https:value},false,now));assert.throws(()=>linkSettings({mtls:value},false,now));
  }
});
test('link issuance freezes settings and original proof, cannot retry or cross any session binding', () => {
  const session=owner(), fields={mode:'short'};
  const attempt=new LinkAttempt(session,workspace,'read',fields,null,false,now);
  fields.mode='never';
  assert.equal(JSON.parse(attempt.requestBody).lifetime.kind,'short');
  assert.equal(attempt.path,'/api/v1/admin/invitations/link');
  assert.equal(attempt.body.includes(invitation),false);
  assert.equal(attempt.request(session).proof,'original-proof');
  assert.throws(()=>attempt.request(session),/Do not retry/);
  for (const change of [s=>s.csrf_token='changed',s=>s.identity.key_id='changed',s=>s.identity.principal_id='changed',
    s=>s.instance_id='changed',s=>s.identity.scope.role='admin',s=>s.identity.scope.workspace_id='changed']) {
    const original=owner(), next=structuredClone(original), pending=new LinkAttempt(original,workspace,'read',{},null,false,now);
    change(next);assert.throws(()=>pending.request(next),/session changed/);
    assert.equal(pending.request(original).proof,original.csrf_token);
  }
});
test('link issuance rejects instance, archived, foreign and unconfirmed group authority before sending',()=>{
  const global=owner();global.identity.scope={kind:'instance'};
  const foreign=owner();foreign.identity.scope.workspace_id='other';
  for (const s of [global,foreign]) assert.throws(()=>new LinkAttempt(s,workspace,'read',{}),/owner\/admin key/);
  assert.throws(()=>new LinkAttempt(owner(),{...workspace,archived:true},'read',{}),/active workspace/);
  assert.throws(()=>new LinkAttempt(owner(),workspace,'write',{}),/owner\/admin key/);
  assert.throws(()=>new LinkAttempt(owner(),workspace,'read',{},unused(),false),/single initial group/);
  assert.throws(()=>new LinkAttempt(owner(),workspace,'read',{}, {...unused(),can_open:false},true),/single initial group/);
  const group=new LinkAttempt(owner(),workspace,'read',{capacity:'2'},unused(),true,now);
  assert.equal(group.kind,'enrollment');assert.equal(JSON.parse(group.requestBody).admission.max_clients,2);
});
test('one-time link reply binds instance, workspace, policy and explicit routing before display',()=>{
  const attempt=new LinkAttempt(owner(),workspace,'read',{},null,false,now);
  assert.equal(attempt.receipt(receipt()).invitation,invitation);
  for (const extra of [{workspace_id:'foreign'},{workspace_name:'foreign'},{invitation_id:'-'.repeat(36)},{invitation_id:'00000000-0000-0000-0000-000000000000'},{max_uses:2},{expires_at:null},{expires_at:1e15},
    {enrollment_link:enrollmentUri({...baseLink,invitation,instance_id:'22222222-2222-4222-8222-222222222222'})},
    {enrollment_link:enrollmentUri({...baseLink,invitation,workspace_id:'22222222-2222-4222-8222-222222222222'})}]) assert.throws(()=>attempt.receipt(receipt(extra)),/could not be verified/);
  const never=new LinkAttempt(owner(),workspace,'read',{mode:'never',noExpiry:true},null,false,now);
  assert.throws(()=>never.receipt(receipt()),/could not be verified/);
  assert.equal(never.receipt(receipt({expires_at:null})).workspace_id,workspace.id);
  const custom=new LinkAttempt(owner(),workspace,'read',{mode:'at',date:localDate(3600)},null,false,now);
  assert.throws(()=>custom.receipt(receipt()),/could not be verified/);
  custom.receipt(receipt({expires_at:Math.floor(now/1000)+3600}));
  const advanced=new LinkAttempt(owner(),workspace,'read',{https:'https://public.example'},null,false,now);
  assert.throws(()=>advanced.receipt(receipt()),/could not be verified/);
});
test('workspace hint formatting controls are visible literal text without changing enrollment metadata',()=>{
  assert.equal(displayLinkName('<script> & Team\u202E\u2028name'),'<script> & Team\\u{202e}\\u{2028}name');
  assert.equal(displayLinkName('名 🦑'), '名 🦑');
  assert.equal(displayLinkName('a\u3164\u034f\u115f\uffa0\u00a0b'), 'a\\u{3164}\\u{34f}\\u{115f}\\u{ffa0}\\u{a0}b');
  assert.equal(displayLinkName('❤️'), '❤️');
});

test('same-document join fragments are scrubbed, replaced privately and unsubscribed on teardown',()=>{
  const events=new EventTarget(), changes=[], scrub=[];
  const location={origin:baseLink.https_origin.replace(/\/$/,''),hash:''};
  const history={replaceState:(...args)=>{scrub.push(args);location.hash='';}};
  const stop=watchJoinLinks(location,history,events,(link,error)=>changes.push({link,error}));
  location.hash=new URL(enrollmentUri(baseLink)).hash;events.dispatchEvent(new Event('hashchange'));
  assert.equal(location.hash,'');assert.equal(changes[0].link.invitation,invitation);
  assert.equal(changes[0].error,'');assert.deepEqual(scrub[0],[null,'','/join']);
  location.hash='#v=99&t='+invitation;events.dispatchEvent(new Event('hashchange'));
  assert.equal(location.hash,'');assert.equal(changes[1].link,null);assert.ok(changes[1].error);
  assert.equal(JSON.stringify(changes[1]).includes(invitation),false);
  location.hash='#key='+invitation;events.dispatchEvent(new Event('hashchange'));
  assert.equal(location.hash,'');assert.equal(changes[2].link,null);
  stop();location.hash='#retained-after-unmount';events.dispatchEvent(new Event('hashchange'));
  assert.equal(changes.length,3);assert.equal(location.hash,'#retained-after-unmount');
});

test('near-seven-day confirmation and group cap allow for a server clock behind the browser',()=>{
  const near=7*86400-120;
  assert.equal(linkDateIsLong(localDate(7*86400-600),now),false);
  assert.equal(linkDateIsLong(localDate(7*86400-599),now),true);
  assert.throws(()=>linkSettings({mode:'at',date:localDate(near)},false,now),/seven days/);
  const settings=linkSettings({mode:'at',date:localDate(near),longLived:true},false,now);
  assert.equal(settings.lifetime.confirm_long_lived,true);
  const serverNow=Math.floor(now/1000)-240;
  assert.ok(settings.lifetime.expires_at>serverNow+7*86400);
  assert.throws(()=>linkSettings({mode:'at',date:localDate(near),longLived:true},true,now),/cap/);
  assert.equal(linkSettings({mode:'at',date:localDate(near),longLived:true,capacity:'5'},true,now).admission.max_clients,5);
});
test('initial group confirmation describes the frozen cap and chosen lifetime',()=>{
  const fields={mode:'short',capacity:'3'};
  const finite=new LinkAttempt(owner(),workspace,'read',fields,unused(),true,now);
  fields.capacity='99';fields.mode='never';
  assert.equal(finite.summary(),'Admission cap: 3. Lifetime: 30 minutes.');
  const uncapped=new LinkAttempt(owner(),workspace,'read',{},unused(),true,now);
  assert.equal(uncapped.summary(),'Admission cap: No count limit. Lifetime: 30 minutes.');
  const never=new LinkAttempt(owner(),workspace,'read',{mode:'never',noExpiry:true,capacity:'2'},unused(),true,now);
  assert.equal(never.summary(),'Admission cap: 2. Lifetime: No expiry.');
  const dated=new LinkAttempt(owner(),workspace,'read',{mode:'at',date:localDate(3600),capacity:'1'},unused(),true,now);
  assert.ok(dated.summary().startsWith('Admission cap: 1. Lifetime: Until '));
  assert.ok(dated.summary().includes(new Date(now+3600000).toLocaleString(undefined,{timeZoneName:'short'})));
});
test('receipt refuses implausible short expiry and binds bounded or uncapped groups and mTLS override',()=>{
  const attempt=new LinkAttempt(owner(),workspace,'read',{},null,false,now);
  assert.throws(()=>attempt.receipt(receipt({expires_at:Math.floor(now/1000)+10*365*86400})),/could not be verified/);
  attempt.receipt(receipt({expires_at:Math.floor(now/1000)+7*86400}));
  assert.throws(()=>attempt.receipt(receipt({expires_at:Math.floor(now/1000)+7*86400+1})),/could not be verified/);
  const group=new LinkAttempt(owner(),workspace,'read',{capacity:'2'},unused(),true,now);
  group.receipt(receipt({max_uses:2}));
  for(const max_uses of [null,1,3]) assert.throws(()=>group.receipt(receipt({max_uses})),/could not be verified/);
  const uncapped=new LinkAttempt(owner(),workspace,'read',{},unused(),true,now);
  uncapped.receipt(receipt({max_uses:null}));
  const mtls=new LinkAttempt(owner(),workspace,'read',{mtls:baseLink.mtls_origin},null,false,now);
  mtls.receipt(receipt());
  assert.throws(()=>mtls.receipt(receipt({enrollment_link:enrollmentUri({...baseLink,invitation,mtls_origin:'https://other.example:8444/'})})),/could not be verified/);
});
