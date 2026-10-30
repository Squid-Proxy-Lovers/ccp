// SPDX-License-Identifier: AGPL-3.0-or-later
// Browser issuance has no replay receipt: freeze the original session and send once.
import {signature, canManageInvitations, enrollmentStatus} from './model.js';
import {parseEnrollmentLink, enrollmentOrigin} from './enrollment_link.js';
const WEEK = 7 * 86400;
// Ask early for confirmation/cap when client and server clocks may differ.
const CLOCK_MARGIN = 600;
export function linkSettings(fields, group = false, now = Date.now()) {
  const mode = fields.mode ?? 'short';
  let lifetime, long = false;
  if (mode === 'short') lifetime = {kind: 'short'};
  else if (mode === 'never') {
    if (fields.noExpiry !== true) throw new Error('Confirm the risk of a link with no expiry.');
    lifetime = {kind: 'never', confirm_no_expiry: true}; long = true;
  } else if (mode === 'at') {
    const seconds = new Date(fields.date).getTime() / 1000;
    if (!Number.isSafeInteger(seconds) || seconds <= now / 1000) throw new Error('Choose a valid future expiry date.');
    long = linkDateIsLong(fields.date, now);
    if (long && fields.longLived !== true) throw new Error('Confirm the risk of a link valid near or beyond seven days.');
    lifetime = {kind: 'at', expires_at: seconds, confirm_long_lived: long};
  } else throw new Error('Choose an invitation lifetime.');
  let admission = {kind: 'individual'};
  if (group) {
    const raw = fields.capacity ?? '';
    const capacity = raw === '' ? null : Number(raw);
    if (capacity !== null && (!Number.isInteger(capacity) || capacity < 1 || capacity > 4294967295)
      || long && capacity === null) throw new Error('Choose a positive admission cap, at most 4294967295. Near-seven-day and longer groups require a cap.');
    admission = {kind: 'initial_group', max_clients: capacity};
  }
  const https_origin = fields.https ? enrollmentOrigin(fields.https) : null;
  const mtls_origin = fields.mtls ? enrollmentOrigin(fields.mtls) : null;
  return {admission, lifetime, https_origin, mtls_origin};
}
export function linkDateIsLong(value, now = Date.now()) {
  return new Date(value).getTime() / 1000 > Math.floor(now / 1000) + WEEK - CLOCK_MARGIN;
}
export function displayLinkName(value) {
  return value.replace(/[\p{Cf}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}\u00a0]/gu,
    char => char === '\ufe0f' ? char : '\\u{' + char.codePointAt(0).toString(16) + '}');
}
export class LinkAttempt {
  #sent = false;
  /** @param {object|null} state */
  constructor(session, workspace, role, fields, state = null, consent = false, now = Date.now()) {
    if (!canManageInvitations(session, workspace) || workspace.archived || !['read', 'read_write'].includes(role)) throw new Error('Use this active workspace’s owner/admin key.');
    if (state !== null) {
      enrollmentStatus(state, workspace.id);
      if (!state.can_open || consent !== true) throw new Error('Confirm the single initial group window after reading its current state.');
    }
    this.kind = state === null ? 'admission' : 'enrollment';
    this.workspace_id = workspace.id;
    this.created_at = Math.floor(now / 1000);
    this.path = '/api/v1/admin/invitations/link';
    this.requestBody = JSON.stringify({workspace_id: workspace.id, role, ...linkSettings(fields, state !== null, now)});
    this.body = JSON.stringify({path: this.path, request: JSON.parse(this.requestBody)});
    this.session = signature(session); this.instance_id = session.instance_id; this.proof = session.csrf_token;
    Object.freeze(this);
  }
  summary() {
    const {admission, lifetime} = JSON.parse(this.requestBody);
    const cap = admission.kind === 'individual' ? 1 : admission.max_clients;
    const duration = lifetime.kind === 'short' ? '30 minutes' : lifetime.kind === 'never'
      ? 'No expiry' : 'Until ' + new Date(lifetime.expires_at * 1000).toLocaleString(undefined, {timeZoneName: 'short'});
    return 'Admission cap: ' + (cap === null ? 'No count limit' : cap) + '. Lifetime: ' + duration + '.';
  }
  request(session) {
    if (this.session !== signature(session)) throw new Error('The session changed. Do not send this link action under the new session.');
    if (this.#sent) throw new Error('Do not retry link issuance. Inspect current invitation metadata first.');
    this.#sent = true;
    return {body: this.requestBody, proof: this.proof};
  }
  receipt(value) {
    const original = JSON.parse(this.requestBody);
    const link = parseEnrollmentLink(value?.enrollment_link);
    const expiry = value.expires_at, max = original.admission.kind === 'individual' ? 1 : original.admission.max_clients;
    if (typeof value.invitation_id !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value.invitation_id)
      || value.invitation_id.replaceAll('-', '') === '0'.repeat(32)
      || value.workspace_id !== this.workspace_id || link.workspace_id !== this.workspace_id
      || link.instance_id !== this.instance_id || link.workspace_name !== value.workspace_name
      || value.max_uses !== max
      || original.lifetime.kind === 'never' && expiry !== null
      || original.lifetime.kind !== 'never' && (!Number.isSafeInteger(expiry) || expiry < 1 || expiry * 1000 > 8640000000000000)
      || original.lifetime.kind === 'short' && expiry > this.created_at + WEEK
      || original.lifetime.kind === 'at' && expiry !== original.lifetime.expires_at
      || original.https_origin !== null && link.https_origin !== original.https_origin
      || original.mtls_origin !== null && link.mtls_origin !== original.mtls_origin) throw new Error('The enrollment link response could not be verified.');
    return link;
  }
}
