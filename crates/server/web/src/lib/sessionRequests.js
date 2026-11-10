// SPDX-License-Identifier: AGPL-3.0-or-later
// Session-bound request ordering, shared by the app and deferred-request tests.
import {releaseAction} from './actionOwner.js';
import {signature} from './model.js';

export async function verifySession(state, deps) {
  if (!state.session || state.locked) throw new Error('Sign in again before continuing.');
  const generation = state.generation;
  const session = state.session;
  let current;
  try { current = await deps.wire('/api/v1/admin/session'); }
  catch (error) {
    if (session === state.session && generation === state.generation) deps.lock(error.message);
    throw error;
  }
  if (session !== state.session) throw new Error('The browser session changed.');
  if (generation !== state.generation) throw new Error('The view changed.');
  if (signature(current) !== signature(session)) {
    deps.lock('Another tab changed this browser session. Existing edits will not be sent under that key.');
    throw new Error('The browser session changed.');
  }
  return current;
}

export async function sessionRequest(state, deps, path, body, confirmed, readOnly = false) {
  // Capture before the first await: adoption can happen even between a
  // successful verify and this function's continuation. Never borrow its CSRF.
  const session = state.session;
  const generation = state.generation;
  await verifySession(state, deps);
  if (session !== state.session) throw new Error('The browser session changed.');
  if (state.locked || generation !== state.generation) throw new Error('The view changed.');
  state.inflight += 1;
  try {
    const data = await deps.wire(path, body === undefined ? undefined : JSON.stringify(body), session.csrf_token);
    if (session !== state.session) throw new Error('The browser session changed.');
    // Acknowledgement is a fact about the write, independent of verification,
    // navigation, or a concurrent lock. Surface secrets and drop the submitted
    // draft synchronously, before any further await or stale-view rejection.
    const recovery = confirmed?.(data) || creationRecovery(path, data);
    try {
      await verifySession(state, deps);
      if (state.locked || generation !== state.generation) throw new Error('The view changed. Refresh before continuing.');
    } catch (error) {
      if (session === state.session && recovery) {
        state.pending = null;
        deps.lock(recovery);
        throw new Error(recovery);
      }
      throw error;
    }
    return data;
  } catch (error) {
    if (session === state.session && (error.status === 401 || error.status === 403)) {
      if (!readOnly || generation === state.generation) deps.lock(error.message);
    }
    throw error;
  } finally { state.inflight -= 1; }
}

export function adoptSession(state, session) {
  releaseAction(state);
  state.activationOwner = null;
  state.activating = false;
  state.generation += 1;
  state.sessionEpoch = (state.sessionEpoch ?? 0) + 1;
  state.session = session;
  state.locked = false;
  state.lockMessage = '';
  // Remount panes so callbacks from the replaced session have no live view.
  // Kept drafts and the owner key live in app state and survive the remount.
  state.refreshTick = (state.refreshTick ?? 0) + 1;
}

export function creationRecovery(path, result) {
  if (path === '/api/v1/admin/workspaces/create' && result?.workspace_id) {
    return `Workspace creation confirmed: ${result.workspace_id}. Copy the owner key shown below privately before signing in again. Do not create a duplicate workspace.`;
  }
  return '';
}
