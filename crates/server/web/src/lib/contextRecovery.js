// SPDX-License-Identifier: AGPL-3.0-or-later
// A closed editor's recovery copy owns a separate draft record. Its controls
// must never clear or copy the draft of an editor opened afterwards.
export function captureContextRecovery(app, key, itemName = '') {
  if (!app.draft) return;
  const text = app.draft;
  const record = app.formDraft(key);
  record.text = text;
  let name = itemName;
  if (!name) {
    try { const candidate = JSON.parse(text).entry?.name; if (typeof candidate === 'string') name = candidate; }
    catch { /* Recovery can also be plain text. */ }
  }
  record.label = name ? `Unsaved text for entry “${name}”` : 'Unsaved text from the context editor';
  app.clean();
}

export function discardContextRecovery(app, key, record) {
  if (app.peekFormDraft(key) !== record) return false;
  app.dropFormDraft(key);
  return true;
}

export function nextContextRecovery(app, prefix, removedKey) {
  const keys = Object.keys(app.drafts).filter(key => key.startsWith(prefix) && app.drafts[key].text);
  const index = keys.indexOf(removedKey);
  return [...keys.slice(index + 1), ...keys.slice(0, index)].find(key => key !== removedKey);
}

export async function focusAfterContextDiscard(app, afterDom, document, target) {
  await afterDom();
  app.effect(() => {
    if (document.activeElement !== document.body) return;
    const next = target.nextKey ? document.getElementById(target.nextKey) : null;
    (next ?? document.getElementById('context-inactive-notice') ?? document.getElementById(target.headingId))?.focus();
  });
}

export async function announceContextRecovery(app, text, afterDom, publish) {
  publish('');
  await afterDom();
  app.effect(() => publish(text));
}

export async function focusRetiredContext(app, hadFocus, afterDom, document) {
  if (!hadFocus) return;
  await afterDom();
  app.effect(() => {
    if (document.activeElement === document.body) document.getElementById('context-inactive-notice')?.focus();
  });
}
