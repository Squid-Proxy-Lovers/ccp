// SPDX-License-Identifier: AGPL-3.0-or-later
// All server/user strings go through textContent or input.value, never HTML.
export const byId = id => document.getElementById(id);
let fieldSerial = 0;
export function el(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
export function button(text, action, className) {
  const node = el('button', text, className);
  node.type = 'button';
  node.addEventListener('click', action);
  return node;
}
export function field(parent, title, {value = '', area = false, readOnly = false} = {}) {
  const label = el('label', title);
  const input = el(area ? 'textarea' : 'input');
  input.id = `ccp-field-${++fieldSerial}`;
  label.htmlFor = input.id;
  input.value = value;
  input.readOnly = readOnly;
  parent.append(label, input);
  return input;
}
export function select(parent, title, items, choose) {
  const label = el('label', title);
  const node = el('select');
  node.id = `ccp-field-${++fieldSerial}`;
  label.htmlFor = node.id;
  node.append(new Option('Choose…', ''));
  for (const item of items) node.append(new Option(item.name, item.id));
  node.addEventListener('change', () => choose(node.value));
  parent.append(label, node);
  return node;
}
export function notice(text) { byId('notice').textContent = text; }
export function stamp(value) { return new Date(value * 1000).toLocaleString(); }
export function actions(parent, ...nodes) {
  const row = el('div', undefined, 'actions');
  row.append(...nodes);
  parent.append(row);
  return row;
}
export function secret(parent, title, value) {
  const box = el('section', undefined, 'warning secret');
  box.append(el('h3', title), el('p', 'Copy this privately before leaving this view. It will not be shown again.'));
  field(box, 'Secret', {value, area: true, readOnly: true});
  actions(box, button('Copy secret', () => copy(value)), button('Dismiss secret', () => box.remove()));
  parent.prepend(box);
}
export async function copy(value) {
  try { await navigator.clipboard.writeText(value); notice('Copied. Keep it private.'); return true; }
  catch { notice('Clipboard unavailable. Select and copy the text manually.'); return false; }
}
