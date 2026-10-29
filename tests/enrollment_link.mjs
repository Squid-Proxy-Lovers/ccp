// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {parseEnrollmentLink, enrollmentUri, enrollmentCommand, takeJoinLink, EnrollmentLinkError} from '../crates/server/web/src/lib/enrollment_link.js';
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
