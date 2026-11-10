// SPDX-License-Identifier: AGPL-3.0-or-later
//
// What this file is: plain source-contract checks for wiring that exists
// only in a .svelte template or a rune-compiled `.svelte.ts` file, so it
// cannot be driven by node:test at all -- there is no Svelte compiler and no
// Vite in this test run, and `$state`/`$derived`/`$effect` are compiler
// macros, not runtime functions. Every test here reads a component's source
// text and asserts a specific call, attribute or class is present.
//
// What this file is not, and does not claim to be: a behavioural test, or a
// substitute for driving the real panel. Every decision that CAN be pulled
// out of a template and expressed as a plain function has been, and is
// tested with real inputs (including deferred-promise races) in
// tests/workspace_panel.mjs; this file only covers what is left after that
// -- the handful of lines that pass a tested function's result into a
// template attribute, a prop, or a reactive effect's DOM call. No browser
// session, mutation run or screenshot is included here or implied by it;
// those are reported separately, outside this repository.
//
// Each remaining check below is annotated with why it is template-only
// rather than something that could have been moved into a tested function.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const read = path => readFile(new URL(`../crates/server/web/src/${path}`, import.meta.url), 'utf8');

const app = await read('lib/app.svelte.ts');
const lockedSignIn = await read('lib/lockedSignIn.js');
const appSvelte = await read('App.svelte');
const pageHeader = await read('components/PageHeader.svelte');
const lockedView = await read('components/LockedView.svelte');
const appSidebar = await read('components/AppSidebar.svelte');
const loginView = await read('components/LoginView.svelte');
const context = await read('views/Context.svelte');
const workspaces = await read('views/Workspaces.svelte');
const createNodeForm = await read('views/context/CreateNodeForm.svelte');

function slice(text, fromMarker, toMarker) {
  const from = text.indexOf(fromMarker);
  assert.notEqual(from, -1, `marker not found: ${fromMarker}`);
  const to = toMarker ? text.indexOf(toMarker, from + fromMarker.length) : text.length;
  assert.notEqual(to, -1, `marker not found: ${toMarker}`);
  return text.slice(from, to);
}

// ---- app.svelte.ts: each method calls the tested function that owns its decision ----
// (app.svelte.ts itself is a rune-compiled class -- its $state fields mean
// the whole file needs the Svelte compiler to load, so none of its methods
// can be imported and called directly from node:test. What each one decides
// is instead tested via the lib/*.js function it delegates to; these checks
// only confirm the delegation itself is still wired up.)

test('[source] app.svelte.ts reuses acquireDraft/dropDraft from drafts.js for its form-draft accessors', () => {
  assert.match(app, /from '\.\/drafts\.js'/);
  assert.match(app, /formDraft\(key: string\): Record<string, string> \{ return acquireDraft\(this\.drafts, key\); \}/);
  assert.match(app, /dropFormDraft\(key: string\): void \{ dropDraft\(this\.drafts, key\); \}/);
});

test('[source] reset() delegates what it clears to clearedSessionState()', () => {
  assert.match(app, /from '\.\/sessionReset\.js'/);
  const reset = slice(app, 'reset(): void {', '\n  }');
  assert.match(reset, /Object\.assign\(this, clearedSessionState\(\)\)/);
});

test('[source] hasUnsavedIdentityWork and beforeunload both read the shared hasUnsavedWork()', () => {
  assert.match(app, /from '\.\/unsavedWork\.js'/);
  assert.match(app, /get hasUnsavedIdentityWork\(\): boolean \{ return hasUnsavedWork\(\{drafts: this\.drafts, createdWorkspace: this\.createdWorkspace\}\); \}/);
  const beforeunload = slice(app, "addEventListener('beforeunload'", ');');
  assert.match(beforeunload, /this\.hasUnsavedIdentityWork/);
});

test('[source] signOut delegates to runSignOut', () => {
  assert.match(app, /from '\.\/identitySwitch\.js'/);
  const signOut = slice(app, 'async signOut(', '\n\n  // ---- workspaces and navigation');
  assert.match(signOut, /runSignOut\(\{/);
  assert.match(signOut, /hasUnsavedWork:\s*\(\)\s*=>\s*this\.hasUnsavedIdentityWork/);
  assert.match(signOut, /confirm:\s*\(\)\s*=>\s*confirmAction\(/);
  // The owner key must count as unsaved work in the sign-out confirm body
  // too, not just in hasUnsavedIdentityWork() -- anchored so the real
  // `this.createdWorkspace !== null` expression can't be swapped for a
  // hardcoded `false`.
  assert.match(signOut, /body: `Signing out discards \$\{unsavedWorkSummary\(draftLabels\(this\.drafts\), this\.createdWorkspace !== null\)\}/);
});

test('[source] login delegates to runLogin', () => {
  const login = slice(app, 'async login(', '\n\n  async signOut');
  assert.match(login, /runLogin\(\{/);
  assert.match(login, /clearForNewIdentity:\s*\(\)\s*=>\s*\{\s*this\.drafts\s*=\s*\{\};\s*this\.createdWorkspace\s*=\s*null;\s*\}/);
  // A failed login must still reach run()'s catch (which shows the error
  // notice): a `.catch(() => undefined)` tacked onto this call would swallow
  // it silently while every substring check above still passes.
  assert.doesNotMatch(login, /\.catch\(/, 'runLogin()\'s rejection must propagate to run(), not be swallowed');
});

test('[source] #hashLogin uses hashSwitchBlocked and runs hashKeySwitch inside run(..., \'login\')', () => {
  const hashLogin = slice(app, '#hashLogin(): void {', '\n\n  /**\n   * A hash-key sign-in');
  assert.match(hashLogin, /hashSwitchBlocked\(this\.busy, this\.locked, this\.mayLeave\(\)\)/);
  // Anchored to start-of-line: a dead-code guard like `if (!1) ` prepended
  // before this call would still satisfy a plain substring/prefix match.
  assert.match(hashLogin, /^\s*void this\.run\(\(\) => this\.hashKeySwitch\(key\), 'login'\);\s*$/m);
});

test('[source] hashKeySwitch delegates to runIdentitySwitch, comparing by identityKey (not signature/CSRF)', () => {
  const hashKeySwitch = slice(app, 'async hashKeySwitch(', '\n\n  async discardAndSignIn(');
  assert.match(hashKeySwitch, /runIdentitySwitch\(\{/);
  assert.match(hashKeySwitch, /sameIdentity:\s*session\s*=>\s*Boolean\(this\.session\)\s*&&\s*identityKey\(session as Session\)\s*===\s*identityKey\(this\.session as Session\)/);
  assert.doesNotMatch(hashKeySwitch, /sameIdentity:[^,]*\bsignature\(/, 'identity comparison must never use signature() (it includes the CSRF token, which always differs)');
  assert.match(hashKeySwitch, /adoptSession:\s*session\s*=>\s*adoptSession\(this, session\)/);
  assert.match(hashKeySwitch, /clearForNewIdentity:\s*\(\)\s*=>\s*\{\s*this\.drafts\s*=\s*\{\};\s*this\.createdWorkspace\s*=\s*null;\s*\}/);
});

test('[source] hashKeySwitch\'s declined notice fires on the plain \'declined\' outcome, not a tautology-narrowed one', () => {
  const hashKeySwitch = slice(app, 'async hashKeySwitch(', '\n\n  async discardAndSignIn(');
  // Anchored to the exact `) notice(` that follows the comparison: a mutation
  // appending `&& outcome !== 'declined'` (always false, since this is the
  // only branch where outcome could be 'declined') would make the call
  // unreachable while every substring check above it still passes.
  assert.match(hashKeySwitch, /else if \(outcome === 'declined'\) notice\(/);
});

test('[source] createWorkspace delegates to createWorkspaceFlow and surfaces the owner key from the confirmed callback too', () => {
  const createWorkspace = slice(app, 'async createWorkspace(', '\n\n  workspaceAcknowledged(');
  assert.match(createWorkspace, /createWorkspaceFlow\(\{/);
  assert.match(createWorkspace, /hasUndismissedOwnerKey:\s*\(\)\s*=>\s*this\.createdWorkspace\s*!==\s*null,\n/, 'must be exactly this comparison, not widened with extra && conditions');
  assert.match(createWorkspace, /drafts: this\.drafts, key: draftKey, submitted,/);
  const confirmedText = slice(createWorkspace, 'confirmedText: data => {', '},\n    });');
  assert.match(confirmedText, /this\.createdWorkspace = data as CreatedWorkspace;/, 'the owner key must surface even when the session check after the write fails');
  assert.match(createWorkspace, /this\.createdWorkspace = result;/, 'and on the plain success path too');
});

test('[source] createWorkspace\'s "Creating workspace…" notice is inside the request closure, not fired for a blocked create', () => {
  const createWorkspace = slice(app, 'async createWorkspace(', '\n\n  workspaceAcknowledged(');
  const requestClosure = slice(createWorkspace, 'request: confirmed => {', '},\n      // Runs on acknowledgement');
  assert.match(requestClosure, /notice\('Creating workspace\./);
  assert.doesNotMatch(createWorkspace, /notice\('Creating workspace\.[\s\S]*createWorkspaceFlow/, 'must not be fired before/outside the request path');
});

test('[source] discardAndSignIn treats an undismissed owner key as something to decide about', () => {
  const fn = slice(app, 'async discardAndSignIn(', '\n  }\n}');
  assert.match(fn, /await discardAndSignIn\(this, confirmAction, \(\) => this\.reset\(\)\)/);
  assert.match(lockedSignIn, /const ownerKey = snapshot\.ownerWorkspaceId !== null;/);
  assert.match(lockedSignIn, /if \(!recovery && !ownerKey && !state\.deletion\)/);
  // An owner key must also rule out the deletion-only dialog wording: that
  // dialog never mentions the owner key, so showing it while one is pending
  // would discard the key with no warning at all.
  assert.match(lockedSignIn, /deletionOnly = Boolean\(state\.deletion && \(!state\.pending \|\| state\.pending === state\.deletion\.reference\)\) && !recovery && !ownerKey;/);
  assert.match(lockedSignIn, /\$\{ownerKey \? ' This also discards the workspace owner key shown below\.' : ''\}/);
});

test('[source] request and verify delegate their complete ordering with the live state', () => {
  const verify = slice(app, 'async verify(', '\n  /** Session/write ordering');
  assert.match(verify, /return verifySession\(this, \{wire, lock: \(message: string\) => this\.lock\(message\)\}\)/);
  const request = slice(app, 'async request<', '\n  /** Serialises');
  assert.match(request, /return sessionRequest\(this, \{wire, lock: \(message: string\) => this\.lock\(message\)\}, path, body, confirmed\)/);
});

// ---- App.svelte: the focus-handoff reactive effect (DOM focus + a local `drawer` write) ----
// This is template-only: `$effect` is a compiler macro, and the call it
// makes is `document.getElementById(...).focus()` -- a real DOM API this
// test run has no DOM for. focusHandoffKey() itself (what the effect keys
// on) is a plain function and is tested directly in workspace_panel.mjs.

test('[source] App.svelte computes the focus key with focusHandoffKey, including activating', () => {
  assert.match(appSvelte, /from '\.\/lib\/focusHandoff\.js'/);
  const call = slice(appSvelte, 'focusHandoffKey(', '\n');
  assert.match(call, /app\.session,\s*app\.locked,\s*app\.activating,/);
});

test('[source] App.svelte\'s focus-handoff effect skips ShellSkeleton and closes a leftover mobile drawer', () => {
  // Exact text, not just presence: a mutation can wrap `drawer = false;` in
  // a dead `if (0)` guard, add an extra early return after the activating
  // check, or drop the `tick()` wrapper around focus() -- all of which still
  // satisfy a loose "contains these substrings somewhere" check.
  const block = slice(appSvelte, 'if (app.activating) return;', '\n  });');
  assert.equal(block, `if (app.activating) return;
    // A mobile drawer left open from a prior session would otherwise still be
    // modal here, and an inert #page-title cannot take focus.
    drawer = false;
    void tick().then(() => document.getElementById('page-title')?.focus({preventScroll: false}));`);
});

// ---- focus-visible ring classes: pure CSS, no logic to extract ----

test('[source] PageHeader and LockedView give the programmatically-focused title a visible, mouse-free ring', () => {
  assert.match(pageHeader, /id="page-title"[^>]*focus-visible:ring-2 focus-visible:ring-ring/);
  assert.match(lockedView, /id="page-title"[^>]*focus-visible:ring-2 focus-visible:ring-ring/);
});

// ---- LockedView: the owner key gets the same Secret display as everywhere else ----

test('[source] LockedView shows an undismissed owner key with the Secret component (reveal/copy, no storage)', () => {
  assert.match(lockedView, /from '\.\.\/lib\/ui\/Secret\.svelte'/);
  assert.match(lockedView, /const ownerKey = \$derived\(app\.createdWorkspace\);/);
  assert.match(lockedView, /\{#if ownerKey\}/);
  assert.match(lockedView, /<Secret title=\{`Workspace owner key for \$\{ownerKey\.workspace_id\}`\} value=\{ownerKey\.owner_key\}/);
});

// ---- busy-key agreement: identitySwitchInFlight/formFrozen key off literal 'signout'/'login' ----
// These are string literals passed to app.run() at the call site; the
// matching strings live in identitySwitch.js's own IDENTITY_SWITCH_KEYS,
// already tested there. This only confirms the two ends still agree.

test('[source] sign-out and login still run under the busy keys identitySwitchInFlight/formFrozen expect', () => {
  assert.match(appSidebar, /app\.signOut\(\), 'signout'\)/);
  assert.match(loginView, /await app\.login\(value\)/);
  assert.match(loginView, /\}, 'login'\);/);
});

// ---- Context.svelte: create() delegates its failure message to reportCreateFailure ----

test('[source] Context.svelte uses creationConfirmedText untouched for the confirmed shelf/book create callback', () => {
  assert.match(context, /from '\.\/context\/creationConfirm\.js'/);
  assert.match(context, /confirmedText: data => creationConfirmedText\(kind, data as T\),$/m);
});

test('[source] Context.svelte\'s create() calls submitDraftCreate with the live drafts store and reportCreateFailure on failure', () => {
  const createFn = slice(context, 'async function create<T', 'const refreshed =');
  assert.match(createFn, /submitDraftCreate\(\{\s*\n\s*drafts: app\.drafts, key: draftKey, submitted,/);
  assert.match(createFn, /request:\s*confirmed\s*=>\s*app\.request<T>\(/);
  assert.doesNotMatch(createFn, /\bapp\.dropFormDraft\(draftKey\)/, 'the identity-unsafe drop must not come back');
  assert.match(createFn, /reportCreateFailure\(\{locked: app\.locked, outcome, kind, message, outcomeUnknown, notify: notice\}\)/);
});

// ---- the shelves pane's badge prop, and the CreateNodeForm draftKey props ----
// These are template attribute values (a prop binding and an each-item
// callback passed to a child component); shelfBadgeLabel/shelfDraftKey/
// bookDraftKey are plain functions and are tested directly.

test('[source] the Shelves pane is wired to shelfBadge (peekFormDraft, never the mutating formDraft)', () => {
  const shelfBadgeLine = slice(context, 'const shelfBadge =', '\n');
  assert.match(shelfBadgeLine, /shelfBadgeLabel\(created\.has\(item\.id\), app\.peekFormDraft\(bookDraftKey\(workspaceId, item\.id\)\)\)/);
  assert.match(context, /id="shelves"[\s\S]{0,400}?badge=\{shelfBadge\}/);
});

test('[source] createShelf/createBook and the matching CreateNodeForm draftKey props use the same key functions', () => {
  const createShelf = slice(context, 'const createShelf =', 'const createBook =');
  assert.match(createShelf, /shelfDraftKey\(workspaceId\)/);
  const createBook = slice(context, 'const createBook =', '// ---- entry writes');
  assert.match(createBook, /bookDraftKey\(workspaceId, parent\.id\)/);
  assert.match(context, /<CreateNodeForm kind="shelf" draftKey=\{shelfDraftKey\(workspaceId\)\}/);
  assert.match(context, /<CreateNodeForm kind="book" draftKey=\{bookDraftKey\(workspaceId, shelf\?\.id \?\? ''\)\}/);
});

// ---- Workspaces.svelte / CreateNodeForm.svelte: discard's result assigned to the local `draft` ----
// discardDraft() now returns the fresh draft itself (or null); the one line
// left here is a plain local-variable assignment (`draft = fresh`) --
// `draft` is Svelte reactive state local to the component and cannot be
// read or asserted on from outside a mounted component.

test('[source] Workspaces.svelte\'s create() captures the draft by reference and checks the result before reacquiring', () => {
  const createFn = slice(workspaces, 'function create(event', 'async function discard(');
  assert.match(createFn, /const submitted = draft;\n/, 'must capture the live object, not a copy');
  assert.doesNotMatch(createFn, /const submitted = \{\s*\.\.\.draft\s*\}/, 'a spread copy breaks identity-safe dropping');
  assert.match(createFn, /app\.createWorkspace\(submitted\.name[\s\S]*?FORM, submitted\)/);
  assert.match(createFn, /if \(!result\) return;/, 'a blocked/failed create (no result) must not reacquire or notify success');
});

test('[source] Workspaces.svelte\'s discard assigns discardDraft\'s returned fresh draft', () => {
  const discardFn = slice(workspaces, 'async function discard(', '</script>');
  assert.match(discardFn, /discardDraft\(\{\s*\n\s*drafts: app\.drafts, key: FORM, current,/);
  assert.match(discardFn, /isBlocked:\s*\(\)\s*=>\s*formFrozen\(app\.busy, app\.busyKey, FORM\)/);
  assert.match(discardFn, /if \(!fresh\) return;\s*\n\s*draft = fresh;/);
  // Anchored so a mutation can't drop the `await tick();` and still pass: the
  // focus call must come strictly after it, not merely appear somewhere in
  // the function.
  const afterFresh = discardFn.slice(discardFn.indexOf('draft = fresh;'));
  assert.match(afterFresh, /await tick\(\);\s*\n\s*document\.getElementById\('create-workspace-name'\)\?\.focus\(\)/);
});

test('[source] Workspaces.svelte disables the submit (not just a toast) while the owner key is undismissed', () => {
  assert.match(workspaces, /const ownerKeyPending = \$derived\(app\.createdWorkspace !== null\);/);
  assert.match(workspaces, /disabled=\{frozen \|\| ownerKeyPending\}/);
  assert.match(workspaces, /\{#if ownerKeyPending\}/);
});

test('[source] Workspaces.svelte freezes every field and both buttons, and the Discard button can actually appear', () => {
  assert.match(workspaces, /const frozen = \$derived\(formFrozen\(app\.busy, app\.busyKey, FORM\)\);/);
  const form = slice(workspaces, '<form class="grid gap-4', '</form>');
  const disabledCount = (form.match(/disabled=\{frozen\}/g) ?? []).length;
  assert.ok(disabledCount >= 2, `expected name and owner to read {frozen}; found ${disabledCount}`);
  assert.match(form, /\{#if hasDraftContent\(draft\)\}/);
  // The total-count check above can still pass if the Discard button's own
  // `disabled` is mutated (e.g. to `frozen && !frozen`, always false) as long
  // as the two inputs still read {frozen}; check the Discard button's own
  // attribute directly.
  assert.match(form, /<Button type="button" variant="ghost" onclick=\{discard\} disabled=\{frozen\}>Discard draft<\/Button>/);
});

test('[source] CreateNodeForm.svelte\'s submit captures the draft before awaiting and passes the live object to oncreate', () => {
  const submitFn = slice(createNodeForm, 'function submit(', 'async function discard(');
  assert.match(submitFn, /const submitted = draft;\n/);
  assert.match(submitFn, /if \(!\(await oncreate\(submitted\)\)\) return;/);
});

test('[source] CreateNodeForm.svelte\'s discard assigns discardDraft\'s returned fresh draft', () => {
  const discardFn = slice(createNodeForm, 'async function discard(', '</script>');
  assert.match(discardFn, /discardDraft\(\{\s*\n\s*drafts: app\.drafts, key: draftKey, current,/);
  assert.match(discardFn, /isBlocked:\s*\(\)\s*=>\s*formFrozen\(app\.busy, app\.busyKey, key\)/);
  assert.match(discardFn, /if \(!fresh\) return;\s*\n\s*draft = fresh;/);
  assert.match(discardFn, /getElementById\(`create-\$\{kind\}-name`\)\?\.focus\(\)/);
});

test('[source] CreateNodeForm.svelte freezes both fields and both buttons', () => {
  assert.match(createNodeForm, /const frozen = \$derived\(formFrozen\(app\.busy, app\.busyKey, key\)\);/);
  const form = slice(createNodeForm, '<form class="mt-3 grid gap-3"', '</form>');
  const disabledCount = (form.match(/disabled=\{frozen\}/g) ?? []).length;
  assert.ok(disabledCount >= 2, `expected name and description to read {frozen}; found ${disabledCount}`);
});

test('[source] CreateNodeForm.svelte offers Create before Discard, like Workspaces.svelte and InviteForm.svelte', () => {
  const form = slice(createNodeForm, '<div class="flex justify-end', '</form>');
  assert.ok(form.indexOf('Create {kind}') < form.indexOf('Discard draft'), 'Create must come before Discard');
});
