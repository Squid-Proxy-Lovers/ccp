// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const html = fs.readFileSync('crates/server/src/admin.html', 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const nodes = new Map();
const requests = [];
const context = vm.createContext({
  document: { getElementById(id) {
    if (!nodes.has(id)) nodes.set(id, { value: id === 'limitFilter' ? '100' : '', style: {}, classList: { toggle() {} }, addEventListener() {} });
    return nodes.get(id);
  } },
  window: { addEventListener() {} },
  localStorage: { getItem() { return ''; } },
  Intl, Date, URLSearchParams, setTimeout() {}, clearTimeout() {}, clearInterval() {}, setInterval() {},
  confirm() { return false; },
  requests,
});
vm.runInContext(script, context);
vm.runInContext(`api = path => new Promise(resolve => requests.push({path, resolve})); renderActivity=()=>{}; renderOverview=()=>{}; toast=()=>{};`, context);
const run = source => vm.runInContext(source, context);
assert.equal(run("parseTime('1770000000').toISOString()"), new Date(1770000000 * 1000).toISOString());
assert.equal(run("parseTime('1770000000123').toISOString()"), new Date(1770000000123).toISOString());
assert.equal(run("parseTime('2026-10-08 10:00:00').toISOString()"), '2026-10-08T10:00:00.000Z');

run("activitySession='first'; oldActivity=loadActivity(); activitySession='second'; newActivity=loadActivity();");
requests.shift().resolve([{session_name: 'first'}]);
await run('oldActivity');
assert.equal(run('activity.length'), 0);
requests.shift().resolve([{session_name: 'second'}]);
await run('newActivity');
assert.equal(run('activity[0].session_name'), 'second');

nodes.get('globalText').value = 'original';
run('globalDirty=true; savingGlobal=saveGlobal(); globalVersion++; globalDirty=true;');
nodes.get('globalText').value = 'edited during save';
requests.shift().resolve({content: 'original', updated_at: '1770000000'});
await run('savingGlobal');
assert.equal(run('globalDirty'), true);
assert.equal(nodes.get('globalText').value, 'edited during save');

run("selected='first'; sessionDirty=true; savingSession=saveSessionMaster(); selected='second'; sessionVersion++; sessionDirty=true;");
requests.shift().resolve({content: 'first saved', updated_at: '1770000000'});
await run('savingSession');
assert.equal(run('sessionDirty'), true);
assert.equal(run('selected'), 'second');

const rejectedSwitch = run("selectSession('third')");
await rejectedSwitch;
assert.equal(run('selected'), 'second');
assert.equal(requests.length, 0);
console.log('Admin regressions passed: epoch timestamps, stale activity, in-flight edit preservation, session save isolation, unsaved navigation.');
