// SPDX-License-Identifier: AGPL-3.0-or-later
// Sign-in, navigation and shared data for the CCP admin panel.
//
// - The admin key lives in memory; it is also kept in localStorage
//   (`ccpAdminKey`, the same slot the classic panel used) only when the
//   operator chooses "Remember on this browser".
// - A navigation bumps `generation`, so responses for an older view are dropped.
// - An unsaved instruction edit (`dirty`) asks before leaving.
import {ApiError, call, enc} from './api';
import {notice, clearToasts} from './notify.svelte';
import {confirmAction} from './ui/confirm.svelte';
import type {AdminOverview, AgentStatus, EntryTarget, Instructions, SessionStats, SessionView, View} from './types';

const KEY_SLOT = 'ccpAdminKey';
const SESSION_SLOT = 'ccpAdminSession';
const LIVE_SLOT = 'ccpAdminLive';
const INSTANCE_VIEWS: View[] = ['sessions', 'live', 'all-agents', 'global-instructions'];

function stored(slot: string): string {
  try { return localStorage.getItem(slot) ?? ''; } catch { return ''; }
}
function store(slot: string, value: string | null): void {
  try { if (value === null) localStorage.removeItem(slot); else localStorage.setItem(slot, value); } catch { /* storage unavailable */ }
}

class App {
  #key = '';
  signedIn = $state(false);
  remembered = $state(false);
  booted = $state(false);
  locked = $state(false);
  lockMessage = $state('');

  sessions = $state.raw<SessionStats[]>([]);
  agents = $state.raw<AgentStatus[]>([]);
  globalMaster = $state.raw<Instructions | null>(null);
  /** Unix seconds of the last successful overview read. */
  loadedAt = $state(0);

  /** Selected session name. */
  session = $state<string | null>(null);
  view = $state<View>('sessions');
  /** Library selection requested from another view. */
  target = $state.raw<EntryTarget | null>(null);

  generation = $state(0);
  refreshTick = $state(0);
  dirty = $state(false);
  busy = $state(false);
  busyKey = $state('');
  inflight = $state(0);
  /** Poll live views (activity, agents) every few seconds. */
  live = $state(stored(LIVE_SLOT) !== 'off');

  get current(): SessionStats | null { return this.sessions.find(item => item.session.session_name === this.session) ?? null; }
  get instanceView(): boolean { return INSTANCE_VIEWS.includes(this.view); }
  get keyHint(): string { return this.#key ? `${this.#key.slice(0, 10)}…${this.#key.slice(-4)}` : ''; }

  // ---- requests -----------------------------------------------------------
  async request<T>(path: string, init: {method?: string; body?: unknown} = {}): Promise<T> {
    if (!this.#key || this.locked) throw new Error('Sign in again before continuing.');
    this.inflight += 1;
    try { return await call<T>(this.#key, path, init); }
    catch (error) {
      if (error instanceof ApiError && error.status === 401) this.lock('The admin key was rejected. It may have been rotated on the server.');
      throw error;
    } finally { this.inflight -= 1; }
  }

  /** Serialises user actions; `busyKey` marks which control is working. */
  async run(action: () => unknown, key = 'action'): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.busyKey = key;
    try { await action(); }
    catch (error) { if (!this.locked) notice((error as Error).message, 'destructive'); }
    finally { this.busy = false; this.busyKey = ''; }
  }
  working(key: string): boolean { return this.busy && this.busyKey === key; }

  // ---- data ---------------------------------------------------------------
  async loadOverview(): Promise<void> {
    const data = await this.request<AdminOverview>('/v1/admin/overview');
    this.sessions = [...data.sessions].sort((a, b) => a.session.session_name.localeCompare(b.session.session_name));
    this.agents = data.agents;
    this.globalMaster = data.global_master;
    this.loadedAt = Date.now() / 1000;
    if (this.session && !this.current) {
      notice(`Session ${this.session} no longer exists.`, 'warning');
      this.session = null;
      store(SESSION_SLOT, null);
      if (!this.instanceView) this.view = 'sessions';
    }
  }

  async createSession(name: string): Promise<void> {
    await this.request('/v1/admin/sessions', {method: 'POST', body: {session_name: name}});
    await this.loadOverview();
    notice(`Session ${name} created.`, 'success');
  }

  async deleteSession(name: string): Promise<void> {
    await this.request(`/v1/admin/sessions/${enc(name)}`, {method: 'DELETE'});
    if (this.session === name) { this.session = null; store(SESSION_SLOT, null); }
    await this.loadOverview();
    notice(`Session ${name} deleted.`, 'success');
  }

  // ---- navigation ---------------------------------------------------------
  async mayLeave(): Promise<boolean> {
    if (!this.dirty) return true;
    const ok = await confirmAction({
      title: 'Discard unsaved instructions?',
      body: 'Your edits have not been saved. Leaving this page drops them.',
      confirm: 'Discard edits',
      destructive: true,
    });
    if (ok) this.dirty = false;
    return ok;
  }

  async navigate(view: View): Promise<void> {
    if (view === this.view) return;
    if (!await this.mayLeave()) return;
    this.generation += 1;
    this.view = view;
  }

  async openSession(name: string, view: SessionView = this.instanceView ? 'overview' : (this.view as SessionView)): Promise<void> {
    if (name === this.session && view === this.view) return;
    if (!await this.mayLeave()) return;
    this.generation += 1;
    this.session = name;
    this.view = view;
    store(SESSION_SLOT, name);
  }

  async openEntry(session: string, target: EntryTarget): Promise<void> {
    if (!await this.mayLeave()) return;
    this.generation += 1;
    this.target = target;
    this.session = session;
    this.view = 'library';
    store(SESSION_SLOT, session);
  }

  toggleLive(): void {
    this.live = !this.live;
    store(LIVE_SLOT, this.live ? null : 'off');
  }

  async refresh(): Promise<void> {
    if (!await this.mayLeave()) return;
    this.generation += 1;
    await this.loadOverview();
    this.refreshTick += 1;
  }

  // ---- sign-in ------------------------------------------------------------
  async login(key: string, remember: boolean): Promise<void> {
    const data = await call<AdminOverview>(key, '/v1/admin/overview');
    this.#key = key;
    this.remembered = remember;
    store(KEY_SLOT, remember ? key : null);
    this.locked = false;
    this.lockMessage = '';
    this.sessions = [...data.sessions].sort((a, b) => a.session.session_name.localeCompare(b.session.session_name));
    this.agents = data.agents;
    this.globalMaster = data.global_master;
    this.loadedAt = Date.now() / 1000;
    const last = stored(SESSION_SLOT);
    if (last && this.sessions.some(item => item.session.session_name === last)) { this.session = last; this.view = 'overview'; }
    else { this.session = null; this.view = 'sessions'; }
    this.generation += 1;
    this.signedIn = true;
  }

  lock(message: string): void {
    this.locked = true;
    this.lockMessage = message;
    this.generation += 1;
    clearToasts();
  }

  signOut(forget = true): void {
    this.#key = '';
    if (forget) store(KEY_SLOT, null);
    this.signedIn = false;
    this.locked = false;
    this.lockMessage = '';
    this.dirty = false;
    this.sessions = [];
    this.agents = [];
    this.globalMaster = null;
    this.target = null;
    this.generation += 1;
    clearToasts();
  }

  async boot(): Promise<void> {
    window.addEventListener('beforeunload', event => { if (this.dirty) { event.preventDefault(); event.returnValue = ''; } });
    // `/admin#key=…` signs in without typing; the fragment never reaches the server.
    const params = new URLSearchParams(location.hash.slice(1));
    const linked = params.get('key') ?? '';
    if (location.hash) history.replaceState(null, '', location.pathname);
    const key = linked || stored(KEY_SLOT);
    try {
      if (key) {
        try { await this.login(key, linked ? false : true); }
        catch (error) {
          if (error instanceof ApiError && error.status === 401) {
            if (!linked) store(KEY_SLOT, null);
            notice(linked ? 'The key in that link was not accepted.' : 'Your saved admin key is no longer accepted.', 'warning');
          } else notice((error as Error).message, 'destructive');
        }
      }
    } finally { this.booted = true; }
  }
}

export const app = new App();
