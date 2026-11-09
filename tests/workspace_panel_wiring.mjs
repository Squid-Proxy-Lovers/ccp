// SPDX-License-Identifier: AGPL-3.0-or-later
//
// What this file is: source-contract checks. Each test reads a .svelte or
// rune-compiled .ts file as plain text and asserts that a specific call,
// argument or class attribute is present in it. None of these tests run a
// component -- they cannot, since node:test has no Svelte compiler and no
// Vite available (`.svelte.ts` and `.svelte` files use `$state`/`$derived`
// macros that only exist once preprocessed) -- so a mutation that changes
// *behaviour* without changing that specific text (for example, a correct
// call wrapped in `if (false && ...)`) is not guaranteed to be caught here.
// `assertLive` below narrows that gap by rejecting a few common dead-code
// patterns around the matched text, but it is a heuristic, not a proof.
//
// What this file is not: a behavioural test suite, and not a substitute for
// actually running the panel. The real behaviour these lines wire up --
// races, confirmations, focus, freezing -- is covered two other ways:
//   1. the pure functions each line delegates to are tested with deferred
//      promises in tests/workspace_panel.mjs, including the exact races
//      from the review (a draft replaced mid-create, a rejected hash-key
//      sign-in, a declined sign-out, etc.);
//   2. a real headless-browser session was driven against the running panel
//      for this task, exercising the scenarios named in the final report.
//      That session is not part of this repository or its test run; this
//      file does not reproduce or stand in for it.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const read = path => readFile(new URL(`../crates/server/web/src/${path}`, import.meta.url), 'utf8');

const app = await read('lib/app.svelte.ts');
const appSvelte = await read('App.svelte');
const pageHeader = await read('components/PageHeader.svelte');
const lockedView = await read('components/LockedView.svelte');
const appSidebar = await read('components/AppSidebar.svelte');
const loginView = await read('components/LoginView.svelte');
const context = await read('views/Context.svelte');
const workspaces = await read('views/Workspaces.svelte');
const createNodeForm = await read('views/context/CreateNodeForm.svelte');

/** Reject a few common ways to make matched code textually present but dead:
 * `if (false ...)`, `... && false`, `... || true)` guards around it. This is
 * a heuristic (it does not parse or execute anything), not a proof that the
 * surrounding code is reachable. */
function assertLive(text, label) {
  assert.doesNotMatch(text, /if\s*\(\s*false\b/, `${label}: must not be gated dead by "if (false"`);
  assert.doesNotMatch(text, /&&\s*false\b/, `${label}: must not be short-circuited dead by "&& false"`);
  assert.doesNotMatch(text, /\|\|\s*true\b/, `${label}: must not be short-circuited dead by "|| true"`);
}

function slice(text, fromMarker, toMarker) {
  const from = text.indexOf(fromMarker);
  assert.notEqual(from, -1, `marker not found: ${fromMarker}`);
  const to = toMarker ? text.indexOf(toMarker, from + fromMarker.length) : text.length;
  assert.notEqual(to, -1, `marker not found: ${toMarker}`);
  return text.slice(from, to);
}

// ---- app.svelte.ts: draft store, sign-out, login, hash-key switch, workspace create ----

test('[source] app.svelte.ts uses the extracted draft store, not a parallel implementation', () => {
  assert.match(app, /from '\.\/drafts\.js'/);
  assert.match(app, /acquireDraft\(/);
  const hasFormDrafts = slice(app, 'get hasFormDrafts()', '\n');
  assert.match(hasFormDrafts, /hasAnyDraftContent\(this\.drafts\)/);
});

test('[source] signOut delegates its whole decision to runSignOut, including the owner key in "unsaved work"', () => {
  assert.match(app, /from '\.\/identitySwitch\.js'/);
  const signOut = slice(app, 'async signOut(', '\n  // ---- workspaces and navigation');
  assertLive(signOut, 'signOut');
  // mayLeave's early return must be immediately followed by the delegation --
  // nothing (e.g. clearing drafts) may run between them.
  assert.match(signOut, /if \(!this\.mayLeave\(\)\) return;\s*\n\s*await runSignOut\(\{/);
  assert.match(signOut, /hasUnsavedWork:\s*\(\)\s*=>\s*this\.hasUnsavedIdentityWork/);
  assert.match(signOut, /confirm:\s*\(\)\s*=>\s*confirmAction\(\{/, 'confirm must show the real dialog, not a bypassed stand-in');
  const hasUnsavedIdentityWork = slice(app, 'get hasUnsavedIdentityWork()', '\n');
  assert.match(hasUnsavedIdentityWork, /this\.hasFormDrafts\s*\|\|\s*this\.createdWorkspace\s*!==\s*null/);
});

test('[source] login delegates to runLogin, clearing drafts and the owner key for the new identity', () => {
  const login = slice(app, 'async login(', '\n\n  async signOut(');
  assertLive(login, 'login');
  assert.match(login, /runLogin\(/);
  assert.match(login, /clearForNewIdentity:\s*\(\)\s*=>\s*\{\s*this\.drafts\s*=\s*\{\};\s*this\.createdWorkspace\s*=\s*null;\s*\}/);
});

test('[source] the hash-key switch runs inside run(..., \'login\') so forms freeze for the whole decision', () => {
  const hashLogin = slice(app, '#hashLogin(): void {', '\n\n  /**\n   * A hash-key sign-in');
  assertLive(hashLogin, '#hashLogin');
  assert.match(hashLogin, /void this\.run\(\(\)\s*=>\s*this\.hashKeySwitch\(key\),\s*'login'\)/);
  // The confirm must be reachable from inside that call, not gated beforehand.
  assert.doesNotMatch(hashLogin, /confirmAction|runIdentitySwitch/, 'the confirm must live inside hashKeySwitch, not before run() freezes the forms');
});

test('[source] hashKeySwitch delegates to runIdentitySwitch, comparing identities by session signature', () => {
  const hashKeySwitch = slice(app, 'async hashKeySwitch(', '\n\n  async discardAndSignIn(');
  assertLive(hashKeySwitch, 'hashKeySwitch');
  assert.match(hashKeySwitch, /runIdentitySwitch\(/);
  assert.match(hashKeySwitch, /sameIdentity:\s*session\s*=>\s*Boolean\(this\.session\)\s*&&\s*signature\(session as Session\)\s*===\s*signature\(this\.session as Session\)/);
  assert.match(hashKeySwitch, /clearForNewIdentity:\s*\(\)\s*=>\s*\{\s*this\.drafts\s*=\s*\{\};\s*this\.createdWorkspace\s*=\s*null;\s*\}/);
  assert.match(hashKeySwitch, /confirm:\s*\(\)\s*=>\s*confirmAction\(\{/, 'confirm must show the real dialog, not a bypassed stand-in');
  assert.match(hashKeySwitch, /outcome === 'same'/);
  assert.match(hashKeySwitch, /outcome === 'declined'/);
});

test('[source] createWorkspace delegates to createWorkspaceFlow, forwards the confirmed callback, and never drops a draft outside it', () => {
  const createWorkspace = slice(app, 'async createWorkspace(', '\n\n  workspaceAcknowledged(');
  assertLive(createWorkspace, 'createWorkspace');
  assert.match(createWorkspace, /createWorkspaceFlow\(/);
  assert.match(createWorkspace, /hasUndismissedOwnerKey:\s*\(\)\s*=>\s*this\.createdWorkspace\s*!==\s*null/);
  assert.match(createWorkspace, /drafts: this\.drafts, key: draftKey, submitted,/, 'must operate on the real draft store, not a disconnected one');
  assert.match(createWorkspace, /request:\s*confirmed\s*=>\s*this\.request<CreatedWorkspace>\(/);
  assert.match(createWorkspace, /\},\s*confirmed\),/, 'confirmed must be forwarded as this.request\'s third argument');
  assert.match(createWorkspace, /this\.createdWorkspace = result;/);
  assert.doesNotMatch(createWorkspace, /\bdropFormDraft\(/, 'no unconditional drop may be added outside createWorkspaceFlow');
});

// ---- App.svelte: focus handoff ----

test('[source] App.svelte computes the focus-handoff key with focusHandoffKey, including activating', () => {
  assert.match(appSvelte, /from '\.\/lib\/focusHandoff\.js'/);
  const call = slice(appSvelte, 'focusHandoffKey(', '\n');
  assert.match(call, /app\.session,\s*app\.locked,\s*app\.activating,/);
});

test('[source] App.svelte\'s focus-handoff effect skips ShellSkeleton and closes a leftover mobile drawer', () => {
  const effect = slice(appSvelte, 'const key = focusHandoffKey', 'document.getElementById(\'page-title\')?.focus(');
  assertLive(effect, 'focus-handoff effect');
  assert.match(effect, /if \(app\.activating\) return;/);
  assert.match(effect, /drawer\s*=\s*false/);
});

test('[source] PageHeader and LockedView give the programmatically-focused title a visible, mouse-free ring', () => {
  assert.match(pageHeader, /id="page-title"[^>]*focus-visible:ring-2 focus-visible:ring-ring/);
  assert.match(lockedView, /id="page-title"[^>]*focus-visible:ring-2 focus-visible:ring-ring/);
});

// ---- busy-key agreement: identitySwitchInFlight/formFrozen key off 'signout'/'login' ----

test('[source] sign-out and login still run under the busy keys identitySwitchInFlight expects', () => {
  assert.match(appSidebar, /app\.signOut\(\), 'signout'\)/);
  assert.match(loginView, /await app\.login\(value\)/);
  assert.match(loginView, /\}, 'login'\);/);
});

// ---- Context.svelte: confirmed-create text, submitDraftCreate wiring, badge, draft keys ----

test('[source] Context.svelte uses creationConfirmedText untouched for the confirmed shelf/book create callback', () => {
  assert.match(context, /from '\.\/context\/creationConfirm\.js'/);
  const confirmedText = slice(context, 'confirmedText: data =>', ',\n    });');
  assertLive(confirmedText, 'Context.svelte confirmedText');
  assert.match(confirmedText, /^confirmedText: data => creationConfirmedText\(kind, data as T\)$/);
});

test('[source] Context.svelte\'s create() calls submitDraftCreate with the live drafts store and forwards confirmed', () => {
  const createFn = slice(context, 'async function create<T', 'const refreshed =');
  assertLive(createFn, 'Context.svelte create()');
  assert.match(createFn, /submitDraftCreate\(\{\s*\n\s*drafts: app\.drafts, key: draftKey, submitted,/);
  assert.match(createFn, /request:\s*confirmed\s*=>\s*app\.request<T>\([^,]+,\s*\{[^}]+\},\s*confirmed\)/s);
  assert.doesNotMatch(createFn, /\bapp\.dropFormDraft\(draftKey\)/, 'the identity-unsafe drop must not come back');
  const failureBranch = slice(createFn, 'if (!outcome.ok)', 'return false;\n    }');
  assert.match(failureBranch, /outcome\.keptCurrent\s*\?/, 'the "kept" claim must read the real outcome, not a hardcoded value');
});

test('[source] the Shelves pane badge peeks (never acquires) the book draft and never shows a plain "New" badge instead', () => {
  const shelfBadgeLine = slice(context, 'const shelfBadge =', '\n');
  assertLive(shelfBadgeLine, 'shelfBadge');
  assert.match(shelfBadgeLine, /shelfBadgeLabel\(created\.has\(item\.id\), app\.peekFormDraft\(bookDraftKey\(workspaceId, item\.id\)\)\)/);
  assert.doesNotMatch(shelfBadgeLine, /app\.formDraft\(/, 'peekFormDraft must never be swapped for the mutating formDraft');
  assert.match(context, /id="shelves"[\s\S]{0,400}?badge=\{shelfBadge\}/);
});

test('[source] createShelf/createBook and the matching CreateNodeForm props use the same key functions', () => {
  const createShelf = slice(context, 'const createShelf =', 'const createBook =');
  assert.match(createShelf, /shelfDraftKey\(workspaceId\)/);
  const createBook = slice(context, 'const createBook =', '// ---- entry writes');
  assert.match(createBook, /bookDraftKey\(workspaceId, parent\.id\)/);
  assert.match(context, /<CreateNodeForm kind="shelf" draftKey=\{shelfDraftKey\(workspaceId\)\}/);
  assert.match(context, /<CreateNodeForm kind="book" draftKey=\{bookDraftKey\(workspaceId, shelf\?\.id \?\? ''\)\}/);
});

// ---- Workspaces.svelte: identity-safe submit/discard, freezing, the owner-key-blocked result ----

test('[source] Workspaces.svelte captures the draft by reference and checks createWorkspace\'s result before reacquiring', () => {
  const createFn = slice(workspaces, 'function create(event', 'async function discard(');
  assertLive(createFn, 'Workspaces.svelte create()');
  assert.match(createFn, /const submitted = draft;\n/, 'must capture the live object, not a copy');
  assert.doesNotMatch(createFn, /const submitted = \{\s*\.\.\.draft\s*\}/, 'a spread copy breaks identity-safe dropping');
  assert.match(createFn, /app\.createWorkspace\(submitted\.name[\s\S]*?FORM, submitted\)/);
  assert.match(createFn, /if \(!result\) return;/, 'a blocked/failed create (no result) must not reacquire or notify success');
  const afterGuard = slice(createFn, 'if (!result) return;', '}, FORM);');
  assert.match(afterGuard, /draft\s*=\s*acquire\(\);/, 'a successful create must reacquire so a later edit is tracked again');
});

test('[source] Workspaces.svelte\'s discard is identity-safe and blocked by the real freeze/busy state', () => {
  const discardFn = slice(workspaces, 'async function discard(', '</script>');
  assertLive(discardFn, 'Workspaces.svelte discard()');
  assert.match(discardFn, /discardDraft\(\{\s*\n\s*drafts: app\.drafts, key: FORM, current,/);
  assert.match(discardFn, /isBlocked:\s*\(\)\s*=>\s*formFrozen\(app\.busy, app\.busyKey, FORM\)/);
  assert.match(discardFn, /confirm:\s*\(\)\s*=>\s*confirmAction\(\{/, 'confirm must show the real dialog, not a bypassed stand-in');
  assert.doesNotMatch(discardFn, /app\.drafts\s*=/, 'discard must never wipe the whole draft store');
  assert.match(discardFn, /getElementById\('create-workspace-name'\)\?\.focus\(\)/);
});

test('[source] Workspaces.svelte freezes every field and both buttons, and the Discard button can actually appear', () => {
  assert.match(workspaces, /const frozen = \$derived\(formFrozen\(app\.busy, app\.busyKey, FORM\)\);/);
  const form = slice(workspaces, '<form class="grid gap-4', '</form>');
  assertLive(form, 'Workspaces.svelte create form');
  const disabledCount = (form.match(/disabled=\{frozen\}/g) ?? []).length;
  assert.ok(disabledCount >= 4, `expected name, owner, submit and discard to all read {frozen}; found ${disabledCount}`);
  assert.match(form, /\{#if hasDraftContent\(draft\)\}/);
});

// ---- CreateNodeForm.svelte: same contract, parameterised by `kind`/`key` ----

test('[source] CreateNodeForm.svelte\'s submit captures the draft before awaiting and passes the live object to oncreate', () => {
  const submitFn = slice(createNodeForm, 'function submit(', 'async function discard(');
  assertLive(submitFn, 'CreateNodeForm.svelte submit()');
  assert.match(submitFn, /const submitted = draft;\n/);
  assert.match(submitFn, /if \(!\(await oncreate\(submitted\)\)\) return;/);
  const afterGuard = slice(submitFn, 'if (!(await oncreate(submitted)))', '}, key);');
  assert.match(afterGuard, /draft\s*=\s*acquire\(\);/, 'a successful create must reacquire so a later edit is tracked again');
});

test('[source] CreateNodeForm.svelte\'s discard is identity-safe and blocked by the real freeze/busy state', () => {
  const discardFn = slice(createNodeForm, 'async function discard(', '</script>');
  assertLive(discardFn, 'CreateNodeForm.svelte discard()');
  assert.match(discardFn, /discardDraft\(\{\s*\n\s*drafts: app\.drafts, key: draftKey, current,/);
  assert.match(discardFn, /isBlocked:\s*\(\)\s*=>\s*formFrozen\(app\.busy, app\.busyKey, key\)/);
  assert.match(discardFn, /confirm:\s*\(\)\s*=>\s*confirmAction\(\{/, 'confirm must show the real dialog, not a bypassed stand-in');
  assert.doesNotMatch(discardFn, /app\.drafts\s*=/, 'discard must never wipe the whole draft store');
  assert.match(discardFn, /getElementById\(`create-\$\{kind\}-name`\)\?\.focus\(\)/);
});

test('[source] CreateNodeForm.svelte freezes both fields and both buttons', () => {
  assert.match(createNodeForm, /const frozen = \$derived\(formFrozen\(app\.busy, app\.busyKey, key\)\);/);
  const form = slice(createNodeForm, '<form class="mt-3 grid gap-3"', '</form>');
  assertLive(form, 'CreateNodeForm.svelte form');
  const disabledCount = (form.match(/disabled=\{frozen\}/g) ?? []).length;
  assert.ok(disabledCount >= 4, `expected name, description, submit and discard to all read {frozen}; found ${disabledCount}`);
});

test('[source] CreateNodeForm.svelte offers Create before Discard, like Workspaces.svelte and InviteForm.svelte', () => {
  const form = slice(createNodeForm, '<div class="flex justify-end', '</form>');
  assert.ok(form.indexOf('Create {kind}') < form.indexOf('Discard draft'), 'Create must come before Discard');
});
