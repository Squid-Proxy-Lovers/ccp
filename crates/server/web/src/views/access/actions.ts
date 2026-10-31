// SPDX-License-Identifier: AGPL-3.0-or-later
// Writes for clients & access. None of these endpoints has an operation ID, so a
// sent request is never retried: a lost or unverifiable response becomes an
// unconfirmed outcome that keeps the original request, and a confirmed one
// re-checks the session before any one-time secret is shown.
import {
  wire, invitationPage, invitationStatus, recoveryPage, recoveryStatus, enrollmentStatus,
  AdmissionAttempt, EnrollmentAttempt, ClientAccessAttempt, SecurityAttempt,
} from '../../lib/model.js';
import {LinkAttempt} from '../../lib/link_issuance.js';
import {app} from '../../lib/app.svelte';
import {notice} from '../../lib/notify.svelte';
import type {Session, Workspace} from '../../lib/types';

export interface Grant {
  id: string; client_id: string; client_name: string; role: string;
  revoked_at: number | null; client_disabled: boolean;
}
export interface GrantPage { grants: Grant[]; next_cursor: string | null }
export interface Admin { id: string; name: string; is_owner: boolean; enabled: boolean }
export interface WorkspaceKey {
  id: string; name: string; principal_id: string; revoked_at: number | null;
  expires_at: number | null; role_active: boolean;
}
export interface InvitationStatus {
  invitation_id: string; uses: number; max_uses: number | null; expires_at: number | null;
  revoked: boolean; expired: boolean; initial_closed: boolean; issuer_authorized: boolean;
  target_client_id: string | null;
}
export interface Invitation { kind: 'initial_group' | 'individual'; role: string; status: InvitationStatus }
export interface Enrollment {
  workspace_id: string; phase: 'unused' | 'open' | 'closed'; closed_at: number | null;
  has_admission_history: boolean; workspace_archived: boolean; can_open: boolean;
  invitation: InvitationStatus | null;
}
export type RecoveryScope = {kind: 'instance'} | {kind: 'workspace'; workspace_id: string};
export interface RecoveryItem {
  invitation_id: string;
  status: {expires_at: number; revoked: boolean; expired: boolean; redeemed: boolean; issuer_authorized: boolean};
}

export interface Attempt {
  kind: string; path: string; body: string; requestBody: string; workspace_id: string;
  request(session: Session): {body: string; proof: string};
}
export interface Observation { note: string; text: string }

/** Labels for one kind of uncertain action, kept close to the original panel's copy. */
export interface Wording {
  pendingTitle: string; pendingBody: string; pendingNotice: string;
  original: string; copyOriginal: string; observe: string;
  forget: string; forgetConfirm: string;
  localTitle: string; localBody: string; localLabel: string; copyLocal: string;
  rereadLocal?: string;
  discardLocal: string; discardConfirm: string;
  discardedTitle: string; discardedBody: string; discardedNotice?: string;
  reread: string;
}

export type Outcome =
  | {phase: 'pending'; wording: Wording; attempt: Attempt; error: string; observe: () => Promise<Observation>; observation?: Observation}
  | {phase: 'local'; wording: Wording; attempt: Attempt}
  | {phase: 'done'; title: string; message: string; secret?: {title: string; value: string}; link?: string; note?: string; reread: string}
  | {phase: 'discarded'; title: string; body: string; reread: string};
export type Report = (outcome: Outcome) => void;

export const roles = [{value: 'read', label: 'Read only'}, {value: 'read_write', label: 'Read / write'}];
export const roleLabel = (role: string) => role === 'read_write' ? 'Read / write' : 'Read only';
export const scopeLabel = (scope: RecoveryScope, workspace: Workspace) => scope.kind === 'instance'
  ? 'Instance-wide · all still-authorized workspaces' : `This workspace only · ${workspace.name} (${workspace.id})`;

const READ_ACCESS = 'Read current client access and invitations';
const UNCANCELLED = 'The server action remains uncancelled.';

export const wording = {
  admission: {
    pendingTitle: 'Invitation action outcome unconfirmed',
    pendingBody: 'This endpoint has no operation ID or safe retry. The action may have completed. Read metadata separately; a newly observed row does not prove which request created it. Secrets cannot be retrieved. Forgetting locally does not cancel delayed execution.',
    pendingNotice: 'Invitation action unconfirmed. Original request retained; no retry sent.',
    original: 'Original invitation action', copyOriginal: 'Copy original invitation action', observe: 'Read current invitation observation',
    forget: 'Forget invitation action locally', forgetConfirm: 'It is not cancelled and may still complete. Keep the original endpoint and target.',
    localTitle: 'Original invitation action retained as a local copy', localBody: UNCANCELLED,
    localLabel: 'Local invitation action copy', copyLocal: 'Copy invitation action',
    discardLocal: 'Discard local invitation copy', discardConfirm: 'Discard this local copy without cancelling the server request?',
    discardedTitle: 'Local invitation copy discarded', discardedBody: 'No server cancellation or current status read is implied.',
    discardedNotice: 'Local invitation copy discarded. Inspect current inventory before another action.',
    reread: READ_ACCESS,
  },
  enrollment: {
    pendingTitle: 'Initial-window action outcome unconfirmed',
    pendingBody: 'No operation ID or replay receipt is available. These controls do not retry; the action may have completed. A window observation is not causal attribution. If the group secret is lost, inspect or revoke its invitation, or close the window and use individual invitations. Do not attempt to reopen it.',
    pendingNotice: 'Initial-window action unconfirmed. No retry was sent; original request retained.',
    original: 'Original initial-window action', copyOriginal: 'Copy original group action', observe: 'Read current initial-window observation',
    forget: 'Forget group action locally', forgetConfirm: 'It is not cancelled and may still complete. Keep the original request.',
    localTitle: 'Original group action retained as a local copy', localBody: UNCANCELLED,
    localLabel: 'Local group action copy', copyLocal: 'Copy group action',
    discardLocal: 'Discard local group copy', discardConfirm: 'Discard the local copy without cancelling the server action?',
    discardedTitle: 'Local group work discarded', discardedBody: 'This does not cancel a server action or imply a current state read.',
    discardedNotice: 'Local group copy discarded. Read current window state before further action.',
    reread: READ_ACCESS,
  },
  clientAccess: {
    pendingTitle: 'Client access action outcome unconfirmed',
    pendingBody: 'No operation ID or replay receipt is available; these controls do not retry. The action may have completed. Current metadata cannot prove which request created a row. Keep the original client, scope and request; secrets cannot be retrieved.',
    pendingNotice: 'Client access action unconfirmed. Original request retained; no retry sent.',
    original: 'Original client access action', copyOriginal: 'Copy original client access action', observe: 'Read current client access observation',
    forget: 'Forget client access action locally', forgetConfirm: 'It is not cancelled and may still complete. Keep a copy of the original.',
    localTitle: 'Original client access action retained as a local copy', localBody: UNCANCELLED,
    localLabel: 'Local client access action copy', copyLocal: 'Copy client access action',
    discardLocal: 'Discard local client access copy', discardConfirm: 'Discard this local copy without cancelling the server action?',
    discardedTitle: 'Local client access work discarded', discardedBody: 'No server cancellation or successful metadata read is implied.',
    reread: 'Read current workspace access',
  },
  security: {
    pendingTitle: 'Security action outcome unconfirmed',
    pendingBody: 'This endpoint has no operation ID or safe retry. The action may have completed. A roster or key observation cannot attribute changes to this request. No secret can be retrieved. Inspect metadata before deliberately issuing another key; do not repeat a new-principal appointment by name.',
    pendingNotice: 'Security action outcome unconfirmed. Original endpoint and target retained; no retry was sent.',
    original: 'Original security action', copyOriginal: 'Copy original security action', observe: 'Read current security metadata',
    forget: 'Forget security action locally', forgetConfirm: 'It is not cancelled and may still complete. Keep a copy of the original endpoint and target.',
    localTitle: 'Original action retained as a local copy', localBody: UNCANCELLED,
    localLabel: 'Local security action copy', copyLocal: 'Copy security action', rereadLocal: 'Discard local copy and read current team',
    discardLocal: 'Discard local security copy', discardConfirm: 'Discard the local copy without reading the server? The original action remains uncancelled.',
    discardedTitle: 'Local security copy discarded', discardedBody: 'The server action remains uncancelled. Read current metadata before another security action.',
    discardedNotice: 'Local security copy discarded. Current server state has not been read.',
    reread: 'Read current administrators & keys',
  },
} satisfies Record<string, Wording>;

// ---- shared flow ------------------------------------------------------------

/** Sends once. Verification failures and lost responses come back as `error`. */
async function send(attempt: Attempt, accept: (result: any) => boolean, refusal: string): Promise<{result: any} | {error: Error}> {
  await app.verify();
  app.pending = attempt;
  try {
    const request = attempt.request(app.session as Session);
    const result = await wire(attempt.path, request.body, request.proof);
    if (!accept(result)) throw new Error(refusal);
    app.pending = null;
    app.clean();
    return {result};
  } catch (error) {
    return {error: error as Error};
  }
}

function unconfirmed(report: Report, words: Wording, attempt: Attempt, error: Error, observe: () => Promise<Observation>): void {
  report({phase: 'pending', wording: words, attempt, error: error.message, observe});
  notice(words.pendingNotice, 'warning');
}

/** A confirmed write must still be followed by a valid session before showing more. */
async function stillSignedIn(lockMessage: string): Promise<boolean> {
  try { await app.verify(); return true; }
  catch { app.lock(lockMessage); return false; }
}

const text = (value: unknown) => JSON.stringify(value, null, 2);
const nonEmpty = (value: unknown) => typeof value === 'string' && value !== '';

// ---- invitations --------------------------------------------------------------

const INVITATIONS = '/api/v1/admin/invitations/';

export async function changeAdmission(report: Report, workspace: Workspace, action: 'individual' | 'revoke', body: Record<string, unknown>): Promise<void> {
  const attempt = new AdmissionAttempt(app.session, workspace, action, body) as Attempt;
  const sent = await send(attempt, result => action === 'individual'
    ? nonEmpty(result?.invitation_id) && nonEmpty(result.secret) : result?.completed === true,
  'The invitation response could not be verified.');
  if ('error' in sent) {
    unconfirmed(report, wording.admission, attempt, sent.error, async () => {
      const original = JSON.parse(attempt.requestBody);
      return original.invitation_id
        ? {note: 'Current status observation; no causal attribution.', text: text(invitationStatus(await app.request(INVITATIONS + 'status', original), original.invitation_id))}
        : {note: 'First page only; no causal attribution. After investigating and discarding locally, use the full inventory pager.', text: text(invitationPage(await app.request(INVITATIONS + 'list', {workspace_id: attempt.workspace_id, limit: 16})))};
    });
    return;
  }
  const {result} = sent;
  const confirmed = action === 'individual'
    ? `Invitation creation confirmed: ${result.invitation_id} in workspace ${body.workspace_id}.`
    : `Invitation revocation confirmed: ${body.invitation_id} in workspace ${body.workspace_id}.`;
  if (!await stillSignedIn(confirmed + (action === 'individual'
    ? ' Its secret is unavailable because this session could not be checked. Sign in with an authorized workspace key to inspect or revoke this invitation before deliberately issuing another.'
    : ' Sign in with an authorized workspace key to inspect current status; revocation is already confirmed.'))) return;
  report({phase: 'done', title: 'Invitation action acknowledged', message: confirmed, reread: READ_ACCESS,
    secret: action === 'individual' ? {title: 'Client invitation · ' + result.invitation_id, value: result.secret} : undefined});
  notice(confirmed + (action === 'individual' ? ' Copy the secret privately before leaving.' : ' Existing client grants and content are preserved.'), 'success');
}

/** Deliberate issuance only: retain an uncertain request without retrying it. */
export async function changeLink(report: Report, attempt: LinkAttempt, ask: () => Promise<boolean> = async () => true): Promise<boolean> {
  if (!await ask()) return false;
  const words = attempt.kind === 'enrollment' ? wording.enrollment : wording.admission;
  const sent = await send(attempt as Attempt, result => { attempt.receipt(result); return true; }, 'The enrollment link response could not be verified.');
  if ('error' in sent) {
    unconfirmed(report, words, attempt as Attempt, sent.error, async () => ({
      note: attempt.kind === 'enrollment'
        ? 'Current initial-window observation only; no causal attribution or cancellation. The private link cannot be retrieved.'
        : 'First page only; no causal attribution. The private link cannot be retrieved. Inspect current state before another issuance.',
      text: text(attempt.kind === 'enrollment' ? await readEnrollment(attempt.workspace_id)
        : invitationPage(await app.request(INVITATIONS + 'list', {workspace_id: attempt.workspace_id, limit: 16}))),
    }));
    return true;
  }
  const {result} = sent;
  const message = `Enrollment link creation confirmed: ${result.invitation_id} in workspace ${attempt.workspace_id}.`;
  if (!await stillSignedIn(message + ' Its private link is unavailable because this session could not be checked. Sign in with an authorized workspace key to inspect or revoke this invitation before deliberately issuing another. An initial group cannot be reopened.')) return true;
  report({phase: 'done', title: 'Enrollment link created', message, link: result.enrollment_link,
    note: result.expires_at === null ? 'No expiry. Revoke unused admissions when this link is no longer needed.' : `Expires: ${new Date(result.expires_at * 1000).toLocaleString()}.`, reread: READ_ACCESS});
  notice('Enrollment link created. Copy it privately before leaving.', 'success');
  return true;
}

export const linkFields = (form: FormDraft) => ({mode: form.get('mode') || 'short', date: form.get('date'),
  noExpiry: form.get('noExpiry') === 'yes', longLived: form.get('longLived') === 'yes', capacity: form.get('capacity'),
  https: form.get('https'), mtls: form.get('mtls')});

// ---- initial group enrollment ---------------------------------------------------

const ENROLLMENT = '/api/v1/admin/enrollment/status';

export async function readEnrollment(workspaceId: string): Promise<Enrollment> {
  return enrollmentStatus(await app.request(ENROLLMENT, {workspace_id: workspaceId}), workspaceId);
}

/** `ask` runs after the attempt validated, so invalid input never reaches a confirmation. */
export async function changeEnrollment(report: Report, state: Enrollment, action: 'initial' | 'close', body: Record<string, unknown>, confirmed: boolean, ask: () => Promise<boolean>): Promise<boolean> {
  const attempt = new EnrollmentAttempt(app.session, state, action, body, confirmed) as Attempt;
  if (!await ask()) return false;
  const opening = action === 'initial';
  const sent = await send(attempt, result => opening
    ? nonEmpty(result?.invitation_id) && nonEmpty(result.secret) : result?.completed === true,
  'The initial-window response could not be verified.');
  if ('error' in sent) {
    unconfirmed(report, wording.enrollment, attempt, sent.error, async () => ({
      note: 'Current observation only; not causal attribution or cancellation.',
      text: text(await readEnrollment(attempt.workspace_id)),
    }));
    return true;
  }
  const {result} = sent;
  const message = opening
    ? `Initial group creation confirmed: ${result.invitation_id} in workspace ${attempt.workspace_id}.`
    : `Initial enrollment closure confirmed in workspace ${attempt.workspace_id}.`;
  if (!await stillSignedIn(message + (opening
    ? ' Its secret is unavailable because the session could not be checked. Inspect or revoke this group invitation, or close the window, then use individual invitations. The group cannot be reopened.'
    : ' Sign in with a valid workspace key to inspect current state.'))) return true;
  report({phase: 'done', title: 'Initial-window action acknowledged', message, reread: READ_ACCESS,
    secret: opening ? {title: 'Initial group invitation · ' + result.invitation_id, value: result.secret} : undefined});
  notice(message + (opening ? ' Copy the secret privately before leaving.' : ' Existing client grants and individual invitations are preserved.'), 'success');
  return true;
}

// ---- client recovery and readmission ------------------------------------------------

const RECOVERY = '/api/v1/admin/recovery/';

export async function readRecovery(clientId: string, scope: RecoveryScope, after: string | null): Promise<{invitations: RecoveryItem[]; next_cursor: string | null}> {
  return recoveryPage(await app.request(RECOVERY + 'list', {client_id: clientId, scope, after, limit: 16}), clientId, scope);
}

/** Targeted readmission is refused while the initial group window is open. */
export async function readmissionBlocked(workspaceId: string): Promise<boolean> {
  const window = await readEnrollment(workspaceId);
  if (window.phase !== 'open') return false;
  notice('Close initial group enrollment before targeted readmission. Use the Enrollment tab to manage that window.', 'warning');
  return true;
}

type ClientChange = {action: 'recover' | 'readmit'; expires_at: number; role?: string} | {action: 'revoke_recovery'; invitation_id: string};

export async function changeClientAccess(report: Report, workspace: Workspace, client: Grant, change: ClientChange, ask: () => Promise<boolean>): Promise<boolean> {
  const attempt = new ClientAccessAttempt(app.session, workspace, client, change) as Attempt & {client_id: string; scope: RecoveryScope; action: string};
  if (!await ask()) return false;
  const sent = await send(attempt, result => attempt.action === 'revoke_recovery'
    ? result?.completed === true : nonEmpty(result?.invitation_id) && nonEmpty(result.secret),
  'The client access response could not be verified.');
  if ('error' in sent) {
    unconfirmed(report, wording.clientAccess, attempt, sent.error, async () => {
      const original = JSON.parse(attempt.requestBody);
      const value = attempt.action === 'readmit'
        ? invitationPage(await app.request(INVITATIONS + 'list', {workspace_id: attempt.workspace_id, limit: 16}))
        : attempt.action === 'revoke_recovery'
          ? recoveryStatus(await app.request(RECOVERY + 'status', original), attempt.client_id, attempt.scope)
          : recoveryPage(await app.request(RECOVERY + 'list', {client_id: attempt.client_id, scope: attempt.scope, limit: 16}), attempt.client_id, attempt.scope);
      return {note: 'Observation only, not attribution or cancellation. Lists are first pages only; use the full pager after local discard.', text: text(value)};
    });
    return true;
  }
  const {result} = sent;
  const id = result.invitation_id || JSON.parse(attempt.requestBody).invitation_id;
  const what = attempt.action === 'readmit' ? 'Readmission invitation creation'
    : attempt.action === 'recover' ? 'Recovery invitation creation' : 'Recovery invitation revocation';
  const confirmed = `${what} confirmed: ${id} for client ${attempt.client_id}. Scope: ${scopeLabel(attempt.scope, workspace)}.`;
  if (!await stillSignedIn(confirmed + (attempt.action === 'readmit'
    ? ' Its secret is unavailable; inspect or revoke this targeted invitation in the workspace inventory before deliberately issuing another.'
    : attempt.action === 'recover'
      ? ' Its secret is unavailable; inspect this exact client and scope recovery history and revoke the undelivered invitation before deliberately issuing another.'
      : ' Current metadata can be read after signing in again; existing recovered access was not revoked.'))) return true;
  report({phase: 'done', title: 'Client access action acknowledged', message: confirmed, reread: wording.clientAccess.reread,
    secret: result.secret ? {title: (attempt.action === 'readmit' ? 'Targeted readmission invitation · ' : 'Recovery invitation · ') + id, value: result.secret} : undefined});
  notice(confirmed + (result.secret ? ' Copy the secret privately before leaving.' : ' Existing credentials and grants remain unchanged.'), 'success');
  return true;
}

// ---- administrators and keys ------------------------------------------------------

export type SecurityAction = 'team/appoint' | 'team/revoke' | 'keys/issue' | 'keys/revoke';
const SECURITY_DONE: Record<SecurityAction, string> = {
  'team/appoint': 'Administrator appointed', 'team/revoke': 'Administrator removed', 'keys/issue': 'Admin key issued', 'keys/revoke': 'Admin key revoked',
};

export async function changeSecurity(report: Report, action: SecurityAction, body: Record<string, any>): Promise<void> {
  const attempt = new SecurityAttempt(app.session, action, body) as Attempt;
  const issuing = action === 'team/appoint' || action === 'keys/issue';
  const sent = await send(attempt, result => issuing
    ? typeof result?.key_id === 'string' && typeof result.principal_id === 'string' && nonEmpty(result.secret)
      && !(action === 'keys/issue' && result.principal_id !== body.principal_id)
      && !(action === 'team/appoint' && body.principal.kind === 'existing' && result.principal_id !== body.principal.principal_id)
    : result?.completed === true,
  issuing ? 'The key creation response could not be verified.' : 'The revocation response could not be verified.');
  if ('error' in sent) {
    unconfirmed(report, wording.security, attempt, sent.error, async () => {
      const admins = await app.request('/api/v1/admin/team/list', {workspace_id: attempt.workspace_id, limit: 16});
      const keys = await app.request('/api/v1/admin/keys/list', {workspace_id: attempt.workspace_id, limit: 16});
      return {note: 'Observed first pages only; this does not confirm this request’s outcome. Use the full paginated view after forgetting locally.', text: text({admins, keys})};
    });
    return;
  }
  const {result} = sent;
  const confirmed = `${SECURITY_DONE[action]}. ` + (result.key_id
    ? `Principal ${result.principal_id}; new key ${result.key_id}. ` : `Target ${body.key_id || body.principal_id}. `);
  if (body.key_id && body.key_id === app.session?.identity.key_id) {
    app.lock(confirmed + 'Your current login key was revoked. Sign in using another owner key, or ask the host operator for local owner-key recovery.');
    return;
  }
  if (!await stillSignedIn(confirmed + (result.key_id
    ? 'Its secret is unavailable because the session could not be checked. Sign in using a valid workspace owner key, inspect keys, issue a replacement for this principal and revoke the undelivered key. Do not repeat appointment by name.'
    : 'Read current metadata after signing in with a valid owner key.'))) return;
  report({phase: 'done', title: 'Security action acknowledged', message: confirmed.trim(), reread: wording.security.reread,
    secret: result.key_id ? {title: 'Workspace admin key · ' + result.key_id, value: result.secret} : undefined,
    note: 'This acknowledges the action; current roles and key state may have changed since. No secret is stored by this panel.'});
  notice((confirmed + (result.key_id ? 'Copy the secret privately before leaving.' : '')).trimEnd(), 'success');
}


// ---- form drafts ------------------------------------------------------------------

/**
 * Creation-form fields backed by `app.formDraft`, so typed values survive
 * navigation. Values equal to their default are not stored, which keeps an
 * untouched form from counting as unsaved work.
 */
export class FormDraft {
  constructor(readonly key: string, readonly defaults: Record<string, string> = {}) {}
  get(name: string): string { return app.drafts[this.key]?.[name] ?? this.defaults[name] ?? ''; }
  set(name: string, value: unknown): void {
    const next = value === null || value === undefined ? '' : String(value);
    if (next === (this.defaults[name] ?? '')) {
      const fields = app.drafts[this.key];
      if (fields) delete fields[name];
    } else app.formDraft(this.key)[name] = next;
  }
  get touched(): boolean { return Object.keys(app.drafts[this.key] ?? {}).length > 0; }
  drop(): void { app.dropFormDraft(this.key); }
}
