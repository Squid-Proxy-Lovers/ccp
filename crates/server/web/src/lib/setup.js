// SPDX-License-Identifier: AGPL-3.0-or-later
// No secret persistence, automatic claim retry, or server-controlled error text.
export function takeSetupCode(location, history) {
  const hash = location.hash;
  if (hash) history.replaceState(null, '', '/admin/setup');
  if (!hash) return '';
  const params = new URLSearchParams(hash.slice(1));
  if (params.size !== 1 || !params.has('code')) throw new Error('Invalid setup link. Paste the code from the latest server startup output.');
  const code = params.get('code') ?? '';
  if (!/^ccp_setup_[0-9a-f]{64}$/.test(code ?? '')) throw new Error('Invalid setup code format. Use the complete startup link.');
  return code;
}
export class SetupError extends Error {
  constructor(message, status = 0, uncertain = false) { super(message); this.status = status; this.uncertain = uncertain; }
}
export async function setupRequest(action, body) {
  if (!['verify', 'claim'].includes(action)) throw new SetupError('Invalid setup action.');
  let response;
  try {
    response = await fetch('/api/v1/setup/' + action, {
      method: 'POST', credentials: 'same-origin', cache: 'no-store', redirect: 'error',
      headers: {'Content-Type': 'application/json', 'X-CCP-CSRF': 'setup'}, body: JSON.stringify(body),
    });
  } catch { throw new SetupError('No confirmed setup response. Check setup state before trying the claim again.', 0, action === 'claim'); }
  let value;
  try { value = await response.json(); }
  catch { throw new SetupError('The setup response could not be verified. Check setup state before continuing.', response.status, action === 'claim'); }
  if (!response.ok || value?.api_version !== 1 || !value.data) {
    const messages = {
      400: 'Check the admin label and workspace name. Each needs 1–256 bytes without control characters.',
      401: 'The setup code was not accepted. Use the latest startup link.',
      403: 'Setup must be opened on this server’s HTTPS address.',
      409: 'Setup is already complete. Use your saved admin key to sign in.',
      410: 'This setup code expired. Restart the server for a new startup link.',
      413: 'The setup request is too large.',
      429: 'Too many incorrect codes. Wait a moment or use the correct startup link.',
      503: 'Setup is busy or temporarily unavailable. Check its state before claiming again.',
    };
    throw new SetupError(messages[response.status] ?? 'Setup could not be verified.', response.status, action === 'claim' && ![400, 401, 403, 409, 410, 413, 429].includes(response.status));
  }
  if (action === 'verify' && (value.data.verified !== true || !validUuid(value.data.instance_id))) {
    throw new SetupError('The setup state response could not be verified. Try checking its state again.');
  }
  return value.data;
}
export function validSetupName(value) {
  const name = value.trim();
  return name.length > 0 && new TextEncoder().encode(name).length <= 256 && !/\p{Cc}/u.test(name);
}
function validUuid(value) { return typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value); }
export function setupKeys(value) {
  if (!value || !['instance_id', 'workspace_id', 'principal_id'].every(k => validUuid(value[k]))
    || !['instance_admin_key', 'workspace_owner_key'].every(k => /^ccp_admin_[0-9a-f]{64}$/.test(value[k]?.secret ?? ''))) {
    throw new SetupError('Setup may have completed, but its key response could not be verified. Check setup state; do not create another owner.', 0, true);
  }
  return value;
}
