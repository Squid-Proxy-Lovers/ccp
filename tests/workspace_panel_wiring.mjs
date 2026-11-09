// SPDX-License-Identifier: AGPL-3.0-or-later
// Source-contract checks: wiring that exists only in a .svelte template or a
// rune-compiled .ts file (so it cannot be driven directly by node:test) is
// verified here by reading the component's source text and asserting the
// specific call, attribute or class it must contain. These are NOT
// behavioural tests -- they do not run the component -- they only guard
// against the call site being removed or pointed at the wrong thing while
// the behaviour itself stays covered by the pure-function tests in
// workspace_panel.mjs. The browser probe in the task report is what actually
// exercises this wiring end to end.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const read = path => readFile(new URL(`../crates/server/web/src/${path}`, import.meta.url), 'utf8');

const app = await read('lib/app.svelte.ts');
const context = await read('views/Context.svelte');
const workspaces = await read('views/Workspaces.svelte');
const createNodeForm = await read('views/context/CreateNodeForm.svelte');

test('[source] app.svelte.ts actually uses the extracted draft store, not a parallel implementation', () => {
  assert.match(app, /from '\.\/drafts\.js'/);
  assert.match(app, /acquireDraft\(/);
  assert.match(app, /dropDraftIfCurrent\(/);
});

test('[source] app.svelte.ts\'s signOut asks confirmIdentitySwitch before discarding kept drafts', () => {
  assert.match(app, /from '\.\/identitySwitch\.js'/);
  const signOut = app.slice(app.indexOf('async signOut('), app.indexOf('// ---- workspaces and navigation'));
  assert.match(signOut, /confirmIdentitySwitch\(/);
});

test('[source] app.svelte.ts\'s hash-key sign-in asks confirmIdentitySwitch, and login() clears drafts', () => {
  const hashLogin = app.slice(app.indexOf('async #hashLogin('), app.indexOf('async discardAndSignIn('));
  assert.match(hashLogin, /confirmIdentitySwitch\(/);
  const login = app.slice(app.indexOf('async login('), app.indexOf('async signOut('));
  assert.match(login, /this\.drafts\s*=\s*\{\}/);
});

test('[source] app.svelte.ts\'s createWorkspace passes a confirmed callback that drops its own draft', () => {
  const createWorkspace = app.slice(app.indexOf('async createWorkspace('), app.indexOf('workspaceAcknowledged('));
  assert.match(createWorkspace, /submitDraftCreate\(/);
  assert.match(createWorkspace, /dropFormDraftIfCurrent\(draftKey, submitted\)/);
  // The confirmed callback (passed as this.request's third argument) must be
  // the thing that drops the draft -- not just the post-success path -- so a
  // confirmed-but-session-lost create still stops offering it as unsaved work.
  const confirmedCallback = createWorkspace.slice(createWorkspace.indexOf('data =>'), createWorkspace.indexOf('}),'));
  assert.match(confirmedCallback, /dropFormDraftIfCurrent\(/);
});

test('[source] Context.svelte uses creationConfirmedText for the confirmed shelf/book create callback', () => {
  assert.match(context, /from '\.\/context\/creationConfirm\.js'/);
  assert.match(context, /creationConfirmedText\(kind, data\)/);
});

test('[source] Context.svelte\'s create() uses submitDraftCreate, not a direct unconditional drop', () => {
  const createFn = context.slice(context.indexOf('async function create<T'), context.indexOf('const refreshed ='));
  assert.match(createFn, /submitDraftCreate\(/);
  assert.doesNotMatch(createFn, /app\.dropFormDraft\(draftKey\)/, 'the identity-unsafe drop must not come back');
});

test('[source] Context.svelte\'s confirmed-create callback drops its own draft, not just the normal-success path', () => {
  const createFn = context.slice(context.indexOf('async function create<T'), context.indexOf('const refreshed ='));
  const confirmedCallback = createFn.slice(createFn.indexOf('(data: T) =>'), createFn.indexOf('creationConfirmedText(kind, data)'));
  assert.match(confirmedCallback, /dropFormDraftIfCurrent\(draftKey, submitted\)/);
});

test('[source] the Shelves pane is wired to the kept-book-draft badge, not a plain "New" badge', () => {
  assert.match(context, /id="shelves"[\s\S]{0,400}?badge=\{shelfBadge\}/);
});

test('[source] Workspaces.svelte re-acquires its draft after a create, via the identity-safe drop', () => {
  const createFn = workspaces.slice(workspaces.indexOf('function create(event'), workspaces.indexOf('async function discard('));
  assert.match(createFn, /app\.createWorkspace\(submitted\.name[\s\S]*?FORM, submitted\)/);
  assert.match(createFn, /draft\s*=\s*acquire\(\)/);
});

test('[source] Workspaces.svelte\'s discard is identity-safe, frozen during sign-in/out, and blocked mid-create', () => {
  const discardFn = workspaces.slice(workspaces.indexOf('async function discard('), workspaces.indexOf('</script>'));
  assert.match(discardFn, /discardDraft\(/);
  assert.match(discardFn, /dropFormDraftIfCurrent\(FORM, current\)/);
  assert.doesNotMatch(discardFn, /app\.drafts\s*=/, 'discard must never wipe the whole draft store');
  assert.match(workspaces, /disabled=\{frozen\}/);
  assert.match(workspaces, /disabled=\{frozen \|\| app\.working\(FORM\)\}/);
});

test('[source] Workspaces.svelte refocuses the name field after a discard', () => {
  const discardFn = workspaces.slice(workspaces.indexOf('async function discard('), workspaces.indexOf('</script>'));
  assert.match(discardFn, /getElementById\('create-workspace-name'\)\?\.focus\(\)/);
});

test('[source] CreateNodeForm.svelte\'s discard is identity-safe, frozen, blocked mid-create, and refocuses the name field', () => {
  const discardFn = createNodeForm.slice(createNodeForm.indexOf('async function discard('), createNodeForm.indexOf('</script>'));
  assert.match(discardFn, /discardDraft\(/);
  assert.match(discardFn, /dropFormDraftIfCurrent\(draftKey, current\)/);
  assert.doesNotMatch(discardFn, /app\.drafts\s*=/, 'discard must never wipe the whole draft store');
  assert.match(discardFn, /getElementById\(`create-\$\{kind\}-name`\)\?\.focus\(\)/);
  assert.match(createNodeForm, /disabled=\{frozen\}/);
});

test('[source] CreateNodeForm.svelte\'s submit captures the draft before awaiting, for identity-safe create', () => {
  const submitFn = createNodeForm.slice(createNodeForm.indexOf('function submit('), createNodeForm.indexOf('async function discard('));
  assert.match(submitFn, /const submitted = draft;/);
  assert.match(submitFn, /oncreate\(submitted\)/);
});

test('[source] CreateNodeForm.svelte offers Create before Discard, like Workspaces.svelte and InviteForm.svelte', () => {
  const form = createNodeForm.slice(createNodeForm.indexOf('<div class="flex justify-end'), createNodeForm.indexOf('</form>'));
  assert.ok(form.indexOf('Create {kind}') < form.indexOf('Discard draft'), 'Create must come before Discard');
});
