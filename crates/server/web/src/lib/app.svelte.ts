// SPDX-License-Identifier: AGPL-3.0-or-later
// Session, navigation and write-safety state for the whole panel.
//
// Invariants carried over from the original panel:
// - every request first re-reads the browser session and locks the page if its
//   signature (instance, key, principal, scope, CSRF token) changed;
// - a navigation bumps `generation`, so responses for an older view are dropped;
// - an uncertain write (`pending`) or unsaved edit (`dirty`) blocks leaving;
// - no key, draft or session data is written to browser storage.
import {identityKey, takeFragment, wire, canWrite, signature} from './model.js';
import {acquireDraft, dropDraft, peekDraft, hasAnyDraftContent, draftLabels, recoveryText as draftsRecoveryText} from './drafts.js';
import {createWorkspaceFlow} from './workspaceCreate.js';
import {runSignOut, runLogin, runIdentitySwitch, hashSwitchBlocked, unsavedWorkSummary} from './identitySwitch.js';
import {hasUnsavedWork} from './unsavedWork.js';
import {clearedSessionState} from './sessionReset.js';
import {notice, clearToasts} from './notify.svelte';
import {confirmAction} from './ui/confirm.svelte';
import type {Session, Workspace, PendingWork, View} from './types';

interface WorkspacePage { workspaces: Workspace[]; next_cursor: string | null }
interface Inspection { workspace: Workspace; owner_id: string | null; [key: string]: unknown }
export interface CreatedWorkspace { workspace_id: string; owner_key: string }

const NO_OPERATION_ID = ['lifecycle', 'security', 'transfer', 'admission', 'enrollment', 'client_access'];

class App {
  session = $state.raw<Session | null>(null);
  workspace = $state.raw<Workspace | null>(null);
  inspection = $state.raw<Inspection | null>(null);
  workspaces = $state.raw<Workspace[]>([]);
  workspacesNext = $state<string | null>(null);
  workspacesAfter = $state<string | null>(null);
  view = $state<View>('overview');
  generation = $state(0);
  dirty = $state(false);
  draft = $state('');
  pending = $state.raw<PendingWork | null>(null);
  locked = $state(false);
  lockMessage = $state('');
  busy = $state(false);
  busyKey = $state('');
  booted = $state(false);
  /** True while a new session loads its workspaces and opens the first view. */
  activating = $state(false);
  /** Requests in flight, for the header progress bar. */
  inflight = $state(0);
  /** Bumped by the header refresh to remount the current view. */
  refreshTick = $state(0);
  /** Workspace deletion tracking survives sign-out until explicitly cleared. */
  deletion = $state.raw<{reference: PendingWork; [key: string]: unknown} | null>(null);
  /** One-time owner key from workspace creation; shown until dismissed. */
  createdWorkspace = $state.raw<CreatedWorkspace | null>(null);
  /** Creation form drafts keyed by form, kept across navigation (not DOM). */
  drafts = $state<Record<string, Record<string, string>>>({});
  #fragmentKey = '';

  get instanceScope(): boolean { return this.session?.identity.scope.kind === 'instance'; }
  writable = (): boolean => Boolean(this.session && this.workspace && canWrite(this.session, this.workspace));

  // ---- drafts -------------------------------------------------------------
  setDraft(text: string): void { this.dirty = true; this.draft = text; }
  clean(): void { this.dirty = false; this.draft = ''; }
  formDraft(key: string): Record<string, string> { return acquireDraft(this.drafts, key); }
  /** Reads a form draft without creating it, for a check that must not mutate during render. */
  peekFormDraft(key: string): Record<string, string> { return peekDraft(this.drafts, key); }
  dropFormDraft(key: string): void { dropDraft(this.drafts, key); }
  get hasFormDrafts(): boolean { return hasAnyDraftContent(this.drafts); }
  /** True once something would be lost by leaving this identity (sign-out,
   * or signing in as a different key): a kept draft, or an undismissed
   * one-time owner key. See lib/unsavedWork.js -- shared with beforeunload
   * and LockedView's own discard decision. */
  get hasUnsavedIdentityWork(): boolean { return hasUnsavedWork({drafts: this.drafts, createdWorkspace: this.createdWorkspace}); }
  /** Unsaved local work as plain text: the pending or edited body, then each form draft by name. */
  recoveryText(): string { return draftsRecoveryText(this.pending, this.draft, this.drafts); }

  mayLeave(): boolean {
    if (this.dirty || this.pending) {
      notice('Save or explicitly discard your draft and resolve the pending write before leaving.', 'warning');
      return false;
    }
    return true;
  }

  // ---- session ------------------------------------------------------------
  lock(message: string): void {
    this.locked = true;
    this.lockMessage = message;
    this.generation += 1;
    // The locked screen states the reason; older toasts no longer apply.
    clearToasts();
  }

  get lockAdvice(): string {
    if (!this.pending && !this.draft) return '';
    if (this.pending?.kind === 'transfer') {
      return 'The transfer has no operation ID or safe retry. Your former-owner keys may already be revoked. Ask the intended new owner to inspect with their key, an instance admin to read the owner metadata, or the host operator to inspect local owner recovery. Forgetting locally does not cancel a transfer.';
    }
    return NO_OPERATION_ID.includes(this.pending?.kind ?? '')
      ? 'This action has no operation ID. Discarding locally does not cancel execution. Copy the original endpoint and target, then read the current metadata before another action.'
      : 'A pending write may already have completed. Discarding here does not cancel it on the server. Keep the original request details to investigate.';
  }

  reset(): void {
    Object.assign(this, clearedSessionState());
    this.generation += 1;
    clearToasts();
  }

  async verify(): Promise<Session> {
    if (!this.session || this.locked) throw new Error('Sign in again before continuing.');
    const generation = this.generation;
    let current: Session;
    try { current = await wire('/api/v1/admin/session'); }
    catch (error) { if (generation === this.generation) this.lock((error as Error).message); throw error; }
    if (generation !== this.generation) throw new Error('The view changed.');
    if (signature(current) !== signature(this.session)) {
      this.lock('Another tab changed this browser session. Existing edits will not be sent under that key.');
      throw new Error('The browser session changed.');
    }
    return current;
  }

  /**
   * `confirmed` describes an acknowledged write. If the session check after the
   * response fails, the page locks with that text instead of treating the
   * write as unconfirmed.
   */
  async request<T = any>(path: string, body?: unknown, confirmed?: (data: T) => string): Promise<T> {
    await this.verify();
    const generation = this.generation;
    const session = this.session as Session;
    this.inflight += 1;
    try {
      const data = await wire(path, body === undefined ? undefined : JSON.stringify(body), session.csrf_token);
      try { await this.verify(); }
      catch (error) {
        const recovery = confirmed?.(data as T) || creationRecovery(path, data);
        if (recovery) { this.pending = null; this.lock(recovery); throw new Error(recovery); }
        throw error;
      }
      if (this.locked || generation !== this.generation) throw new Error('The view changed. Refresh before continuing.');
      return data as T;
    } catch (error) {
      const status = (error as {status?: number}).status;
      if (status === 401 || status === 403) this.lock((error as Error).message);
      throw error;
    } finally { this.inflight -= 1; }
  }

  /** Serialises user actions. Controls stay focusable; `busyKey` marks which one is working. */
  async run(action: () => unknown, key = 'action'): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.busyKey = key;
    try { await action(); }
    catch (error) { if (!this.locked) notice((error as Error).message, 'destructive'); }
    finally { this.busy = false; this.busyKey = ''; }
  }
  working(key: string): boolean { return this.busy && this.busyKey === key; }

  async activate(session: Session): Promise<void> {
    this.activating = true;
    try { await this.#activate(session); } finally { this.activating = false; }
  }

  async #activate(session: Session): Promise<void> {
    this.generation += 1;
    this.session = session;
    this.locked = false;
    this.lockMessage = '';
    this.workspace = null;
    this.inspection = null;
    this.view = session.identity.scope.kind === 'instance' ? 'workspaces' : 'overview';
    await this.loadWorkspaces();
    if (session.identity.scope.kind === 'workspace') {
      const scoped = this.workspaces.find(item => item.id === session.identity.scope.workspace_id) ?? this.workspaces[0];
      if (scoped) await this.openWorkspace(scoped, 'overview');
    }
    notice('Signed in. Each action checks your current key permissions.', 'success');
  }

  /** A new identity must never see a previous one's creation drafts or
   * owner key. Only clears them once the session request actually succeeds
   * (never before), so a rejected key leaves the current identity's forms
   * exactly as they were -- nothing orphaned, nothing silently lost. */
  async login(key: string): Promise<void> {
    await runLogin({
      request: () => wire('/api/v1/admin/session/login', JSON.stringify({key}), 'login'),
      clearForNewIdentity: () => { this.drafts = {}; this.createdWorkspace = null; },
      activate: session => this.activate(session as Session),
    });
  }

  async signOut(): Promise<void> {
    if (!this.mayLeave()) return;
    await runSignOut({
      hasUnsavedWork: () => this.hasUnsavedIdentityWork,
      confirm: () => confirmAction({
        title: 'Discard local work?',
        body: `Signing out discards ${unsavedWorkSummary(draftLabels(this.drafts), this.createdWorkspace !== null)}. Copy anything you need first.`,
        confirm: 'Discard and sign out', destructive: true,
      }),
      verify: () => this.verify(),
      logout: session => wire('/api/v1/admin/session/logout', '{}', (session as Session).csrf_token),
      reset: () => {
        this.reset();
        notice('Signed out of this browser session. Your underlying key remains valid until revoked or expired.');
      },
    });
  }

  // ---- workspaces and navigation -----------------------------------------
  async loadWorkspaces(after: string | null = null): Promise<void> {
    const query = new URLSearchParams({limit: '32'});
    if (after) query.set('after', after);
    const data = await this.request<WorkspacePage>(`/api/v1/admin/workspaces?${query}`);
    this.workspaces = data.workspaces;
    this.workspacesNext = data.next_cursor;
    this.workspacesAfter = after;
  }

  async openWorkspace(workspace: Workspace, view: View = this.view === 'workspaces' || this.view === 'global-guidance' ? 'overview' : this.view): Promise<void> {
    this.generation += 1;
    if (workspace.id !== this.workspace?.id) this.inspection = null;
    this.workspace = workspace;
    this.view = view;
    await this.inspectWorkspace();
  }

  async inspectWorkspace(): Promise<void> {
    if (!this.workspace) return;
    const info = await this.request<Inspection>('/api/v1/admin/workspaces/inspect', {workspace_id: this.workspace.id});
    this.workspace = info.workspace;
    this.inspection = info;
  }

  /** Switch view inside the current workspace (or a global view). */
  async navigate(view: View): Promise<void> {
    if (view === this.view) return;
    if (!this.mayLeave()) return;
    this.generation += 1;
    this.view = view;
    if (view === 'global-guidance' || view === 'workspaces') return;
    if (this.workspace) await this.inspectWorkspace();
  }

  /**
   * `submitted` is the exact draft object the caller read `name`/`owner`
   * from; it is only dropped if it is still the live draft for `draftKey`
   * once this resolves (or, if the session check right after a confirmed
   * create fails, from inside that confirmed callback) -- never a
   * replacement draft typed while this was in flight. Blocked (returns
   * `null`) while the previous create's one-time owner key is still on
   * screen: creating a second workspace would otherwise silently overwrite
   * and lose it, and there is no safe default to ask for instead.
   */
  async createWorkspace(name: string, owner: string, draftKey: string, submitted: Record<string, string>): Promise<CreatedWorkspace | null> {
    const outcome = await createWorkspaceFlow({
      hasUndismissedOwnerKey: () => this.createdWorkspace !== null,
      drafts: this.drafts, key: draftKey, submitted,
      // Only reached once the owner-key block is known not to apply, so this
      // never fires a "Creating workspace…" toast for a request never sent.
      request: confirmed => {
        notice('Creating workspace. If the response is lost, refresh the list before repeating.');
        return this.request<CreatedWorkspace>('/api/v1/admin/workspaces/create', {
          name, owner: {kind: 'new_principal', name: owner}, key_expires_at: null,
        }, confirmed);
      },
      // Runs whether the create finishes normally or the session check right
      // after it fails: an acknowledged create's owner key must surface
      // either way, never only on the happy path.
      confirmedText: data => {
        this.createdWorkspace = data as CreatedWorkspace;
        return creationRecovery('/api/v1/admin/workspaces/create', data as {workspace_id?: string});
      },
    });
    if (outcome.blocked) return null;
    if (!outcome.ok) throw outcome.error;
    const result = outcome.result as CreatedWorkspace;
    this.createdWorkspace = result;
    try { await this.loadWorkspaces(); }
    catch (error) {
      if (this.locked) this.lock(creationRecovery('/api/v1/admin/workspaces/create', result));
      else notice(`Workspace created. Copy its owner key privately before navigating away. Workspace list refresh failed: ${(error as Error).message}`, 'warning');
    }
    return result;
  }

  workspaceAcknowledged(archived: boolean): void {
    if (this.workspace) this.workspace = {...this.workspace, archived};
  }
  workspaceObserved(archived: boolean): void {
    if (this.workspace) this.workspace = {...this.workspace, archived};
  }

  // ---- boot ---------------------------------------------------------------
  async boot(): Promise<void> {
    this.#fragmentKey = takeFragment(location, history);
    window.addEventListener('beforeunload', event => {
      if (this.dirty || this.pending || this.deletion || this.hasUnsavedIdentityWork) { event.preventDefault(); event.returnValue = ''; }
    });
    window.addEventListener('focus', () => {
      if (this.session && !this.locked && !this.busy) void this.run(() => this.verify(), 'verify');
    });
    window.addEventListener('hashchange', () => this.#hashLogin());
    await this.run(async () => {
      try {
        if (this.#fragmentKey) {
          const key = this.#fragmentKey;
          this.#fragmentKey = '';
          await this.login(key);
        } else {
          try { await this.activate(await wire('/api/v1/admin/session')); }
          catch (error) { if ((error as {status?: number}).status !== 401) notice((error as Error).message, 'destructive'); }
        }
      } finally { this.booted = true; }
    }, 'boot');
  }

  #hashLogin(): void {
    if (!new URLSearchParams(location.hash.slice(1)).has('key')) return;
    const key = takeFragment(location, history);
    if (!key) { notice('Invalid login fragment. Enter your key manually.', 'warning'); return; }
    if (hashSwitchBlocked(this.busy, this.locked, this.mayLeave())) {
      notice('The login link was removed from this URL. Finish or discard local work, then open your saved link again.', 'warning');
      return;
    }
    // Runs the ask (if there is unsaved work), the request and the identity
    // comparison all under busyKey 'login', so forms stay frozen for the
    // whole decision and a window-focus verify cannot start while the
    // confirm dialog is open or claim the single busy slot out from under it.
    void this.run(() => this.hashKeySwitch(key), 'login');
  }

  /**
   * A hash-key sign-in while already signed in. The key is opaque, so which
   * identity it holds cannot be known before the request; if there is
   * unsaved work, this asks BEFORE sending anything, since the server
   * replaces the session cookie on every successful login (same identity or
   * not) and a decline must truly send nothing. Once sent, a successful
   * result is compared by stable identity only (never the CSRF/session
   * token, which always differs): the same identity adopts the new
   * session/CSRF and keeps every draft and the owner key; a different one
   * clears them, then activates. See lib/identitySwitch.js.
   */
  async hashKeySwitch(key: string): Promise<void> {
    const outcome = await runIdentitySwitch({
      hasUnsavedWork: () => this.hasUnsavedIdentityWork,
      confirm: () => confirmAction({
        title: 'Replace the current session?',
        body: `Signing in with this link replaces your current session and discards ${unsavedWorkSummary(draftLabels(this.drafts), this.createdWorkspace !== null)}. Copy anything you need first.`,
        confirm: 'Sign in and discard', destructive: true,
      }),
      request: () => wire('/api/v1/admin/session/login', JSON.stringify({key}), 'login'),
      sameIdentity: session => Boolean(this.session) && identityKey(session as Session) === identityKey(this.session as Session),
      adoptSession: session => { this.session = session as Session; },
      clearForNewIdentity: () => { this.drafts = {}; this.createdWorkspace = null; },
      activate: session => this.activate(session as Session),
    });
    if (outcome === 'same') notice('Already signed in with this key.');
    else if (outcome === 'declined') notice('The login link was removed from this URL. Finish or discard local work, then open your saved link again.', 'warning');
  }

  async discardAndSignIn(): Promise<void> {
    const recovery = this.recoveryText();
    const ownerKey = this.createdWorkspace !== null;
    if (!recovery && !ownerKey && !this.deletion) { this.reset(); return; }
    const deletionOnly = Boolean(this.deletion && (!this.pending || this.pending === this.deletion.reference)) && !recovery && !ownerKey;
    const ok = await confirmAction(deletionOnly
      ? {title: 'Return to sign in?', body: 'Deletion tracking stays on this page. Any server request remains uncancelled.', confirm: 'Return to sign in'}
      : {
        title: 'Discard local work?',
        body: `Any uncertain server write remains uncertain.${ownerKey ? ' This also discards the workspace owner key shown below.' : ''} Copy your work first if you need it.`,
        confirm: 'Discard and sign in', destructive: true,
      });
    if (ok) this.reset();
  }
}

function creationRecovery(path: string, result: {workspace_id?: string} | undefined): string {
  if (path === '/api/v1/admin/workspaces/create' && result?.workspace_id) {
    return `Workspace creation confirmed: ${result.workspace_id}. The owner key is no longer displayed because this session could not be checked. Ask the CCP host operator to recover this workspace's owner key using the local reset-owner-key command. This requires host access; an instance key cannot reset it remotely. Do not create a duplicate workspace.`;
  }
  return '';
}

export const app = new App();
