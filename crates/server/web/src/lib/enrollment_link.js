// SPDX-License-Identifier: AGPL-3.0-or-later
// Pure link handling only. Never send a browser invitation to any endpoint.
const fields = ['v', 'i', 'm', 'ca', 't', 'w', 'n'];
const messages = {
  size: 'Enrollment link exceeds the size limit.',
  syntax: 'Enrollment link contains invalid encoding or characters.',
  version: 'This enrollment link version is not supported.',
  origin: 'Enrollment links require HTTPS origins without credentials or queries.',
  fields: 'Enrollment link has missing, repeated or unknown fields.',
  identity: 'Enrollment link has an invalid instance or workspace UUID.',
  pin: 'Enrollment link requires a SHA-256 CA certificate pin.',
  invitation: 'Enrollment link has an invalid invitation format.',
  name: 'Enrollment link has an invalid workspace name.',
};
export class EnrollmentLinkError extends Error {
  constructor(kind) { super(messages[kind] ?? messages.syntax); this.kind = kind; }
}
function refuse(kind) { throw new EnrollmentLinkError(kind); }
function utf8(value) { return new TextEncoder().encode(value); }
function id(value) {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value)
    || value.replaceAll('-', '') === '0'.repeat(32)) refuse('identity');
  return value;
}
function origin(value) {
  if (utf8(value).length > 2048 || !value.startsWith('https://') || /[^\x00-\x7f]/u.test(value) || /[@\\%?#\p{Cc}\t\n\f\r ]/u.test(value)) refuse('origin');
  const raw = value.slice(8);
  const slash = raw.indexOf('/');
  if (!raw || raw.startsWith('/') || (slash !== -1 && raw.slice(slash) !== '/')) refuse('origin');
  const hostInput = rawHost(raw.replace(/\/$/, ''));
  let url;
  try { url = new URL(value); } catch { refuse('origin'); }
  if (url.protocol !== 'https:' || url.pathname !== '/' || url.search || url.hash || url.port === '0' || hostInput.toLowerCase() !== url.hostname) refuse('origin');
  const host = url.hostname;
  if (host.startsWith('[')) { if (host === '[::]' || /^\[::ffff:/i.test(host)) refuse('origin'); }
  else if (/^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$/.test(host)) {
    if (host === '0.0.0.0') refuse('origin');
  } else if (host.length > 253 || host.split('.').some(label => label.slice(2, 4) === '--' || !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i.test(label))) refuse('origin');
  return url.href;
}
function rawHost(authority) {
  let host, port;
  if (authority.startsWith('[')) {
    const end = authority.indexOf(']');
    if (end === -1) refuse('origin');
    host = authority.slice(0, end + 1);
    const suffix = authority.slice(end + 1);
    if (suffix) { if (!suffix.startsWith(':')) refuse('origin'); port = suffix.slice(1); }
  } else {
    const colon = authority.indexOf(':');
    host = colon === -1 ? authority : authority.slice(0, colon);
    if (colon !== -1) port = authority.slice(colon + 1);
  }
  if (port !== undefined && (!/^[1-9][0-9]{0,4}$/.test(port) || Number(port) > 65535)) refuse('origin');
  return host;
}
function mtls(https, value) {
  if (value.startsWith('https://')) return origin(value);
  if (!/^[1-9][0-9]{0,4}$/.test(value) || Number(value) > 65535) refuse('origin');
  let url;
  try { url = new URL(https); url.port = value; } catch { refuse('origin'); }
  return origin(url.href);
}
function pin(value) {
  if (!/^sha256:[A-Za-z0-9_-]{43}$/.test(value)) refuse('pin');
  const data = value.slice(7);
  const binary = atob(data.replaceAll('-', '+').replaceAll('_', '/') + '=');
  const canonical = btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
  if (binary.length !== 32 || canonical !== data) refuse('pin');
  return value;
}
export function parseEnrollmentLink(value) {
  try {
    if (typeof value !== 'string') refuse('syntax');
    if (utf8(value).length > 8192) refuse('size');
    if (/[\\\p{Cc}\t\n\f\r ]/u.test(value)) refuse('syntax');
    for (const char of value) {
      const code = char.codePointAt(0);
      if (code >= 0xd800 && code <= 0xdfff) refuse('syntax');
    }
    const hash = value.indexOf('#');
    if (hash === -1) refuse('fields');
    const base = value.slice(0, hash), fragment = value.slice(hash + 1);
    if (fragment.includes('#')) refuse('origin');
    const slash = base.lastIndexOf('/');
    if (base.slice(slash) !== '/join') refuse('origin');
    const https = origin(base.slice(0, slash));
    const parts = new Map();
    for (const pair of fragment.split('&')) {
      const equals = pair.indexOf('=');
      if (equals === -1) refuse('fields');
      const decode = part => decodeURIComponent(part.replaceAll('+', ' '));
      const key = decode(pair.slice(0, equals));
      if (!fields.includes(key)) refuse('fields');
      const decoded = decode(pair.slice(equals + 1));
      if (parts.has(key)) refuse('fields');
      parts.set(key, decoded);
    }
    if (parts.size !== 7) refuse('fields');
    if (parts.get('v') !== '1') refuse('version');
    const instance_id = id(parts.get('i')), workspace_id = id(parts.get('w'));
    const mtls_origin = mtls(https, parts.get('m')), ca_pin = pin(parts.get('ca'));
    const name = parts.get('n');
    // Rust's trim uses Unicode White_Space; JS trim also strips U+FEFF.
    const trimmed = name.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '');
    if (!name || utf8(name).length > 256 || trimmed !== name || /\p{Cc}/u.test(name)) refuse('name');
    const invitation = parts.get('t');
    if (!/^ccp_join_[0-9a-f]{64}$/.test(invitation)) refuse('invitation');
    const link = {https_origin: https, mtls_origin, instance_id, workspace_id, workspace_name: name, ca_pin};
    Object.defineProperty(link, 'invitation', {value: invitation, enumerable: false});
    return Object.freeze(link);
  } catch (error) {
    if (error instanceof EnrollmentLinkError) throw error;
    throw new EnrollmentLinkError('syntax');
  }
}
export function enrollmentUri(link) {
  const params = new URLSearchParams([
    ['v', '1'], ['i', link.instance_id], ['m', link.mtls_origin],
    ['ca', link.ca_pin], ['t', link.invitation], ['w', link.workspace_id], ['n', link.workspace_name],
  ]);
  // Canonical encoding removes raw shell metacharacters from display-name hints.
  const uri = link.https_origin + 'join#' + params;
  parseEnrollmentLink(uri);
  return uri;
}
// POSIX sh/bash/zsh only. Other shells should use the hidden-paste flow.
export function enrollmentCommand(link) {
  const uri = enrollmentUri(link);
  return "ccp enroll '" + uri.replaceAll("'", "'\\''") + "'";
}
export function takeJoinLink(location, history) {
  const hash = location.hash;
  if (hash) history.replaceState(null, '', '/join');
  return hash ? parseEnrollmentLink(location.origin + '/join' + hash) : null;
}

// Reuse the strict v1 origin rules for public-address issuer controls.
export const enrollmentOrigin = origin;

// Same-document link navigation does not remount the public page. Always scrub
// the new fragment before parsing it, including malformed replacement links.
/** @param {(link: ReturnType<typeof parseEnrollmentLink>|null, error: string) => void} update */
export function watchJoinLinks(location, history, events, update) {
  const receive = () => {
    try { update(takeJoinLink(location, history), ''); }
    catch (error) { update(null, error instanceof EnrollmentLinkError ? error.message : 'Invalid enrollment link.'); }
  };
  events.addEventListener('hashchange', receive);
  return () => events.removeEventListener('hashchange', receive);
}
