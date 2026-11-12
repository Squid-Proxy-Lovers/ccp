// SPDX-License-Identifier: AGPL-3.0-or-later
// Validate source observations before display; never interpret them as authority.
const encoder = new TextEncoder();
const fail = () => { throw new Error('Source history could not be verified. Use the CLI to inspect this entry.'); };
const check = value => { if (!value) fail(); };
const fields = (value, names) => check(value && !Array.isArray(value) && typeof value === 'object'
  && Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name)));
const integer = value => typeof value === 'string' && value.length <= 20 && /^-?(0|[1-9][0-9]*)$/.test(value) && value !== '-0' && BigInt(value) >= -(2n ** 63n) && BigInt(value) < 2n ** 63n;
const ordinal = value => Number.isSafeInteger(value) && value > 0;
const hex = (value, size) => typeof value === 'string' && value.length === size && /^[0-9a-f]+$/.test(value);
const tables = new Set(['message_packs', 'message_history', 'deleted_message_packs', 'deleted_message_history']);

function scalar(value) {
  fields(value, value?.type === 'null' ? ['type'] : ['type', 'value']);
  switch (value.type) {
    case 'null': return;
    case 'integer': check(integer(value.value)); return;
    case 'real': check(hex(value.value, 16)); return;
    case 'text': check(typeof value.value === 'string'); return;
    case 'blob': case 'text_bytes':
      check(Array.isArray(value.value) && value.value.every(n => Number.isInteger(n) && n >= 0 && n <= 255)); return;
    default: fail();
  }
}

function record(value) {
  fields(value, ['ordinal', 'source_table', 'source_key', 'row']);
  check(ordinal(value.ordinal) && tables.has(value.source_table));
  check(value.source_table === 'deleted_message_packs'
    ? typeof value.source_key === 'string' && encoder.encode(value.source_key).length > 0 && encoder.encode(value.source_key).length <= 8192
    : integer(value.source_key));
  const row = value.row;
  fields(row, ['columns', 'values']);
  check(Array.isArray(row.columns) && row.columns.length > 0 && row.columns.length <= 256
    && row.columns.every(name => typeof name === 'string') && new Set(row.columns).size === row.columns.length
    && Array.isArray(row.values) && row.columns.length === row.values.length);
  for (const value of row.values) scalar(value);
  check(encoder.encode(JSON.stringify(row)).length <= 2 * 1024 * 1024);
}

export function nativeHistoryPage(value, workspace, entry, after = null) {
  check(after === null || Number.isSafeInteger(after) && after >= 0);
  fields(value, ['workspace_id', 'entry_id', 'source', 'items', 'next_cursor']);
  check(value.workspace_id === workspace && value.entry_id === entry && Array.isArray(value.items) && value.items.length <= 8);
  if (value.source === null) {
    check(value.items.length === 0 && value.next_cursor === null);
    return value;
  }
  fields(value.source, ['database_sha256', 'schema', 'converted_at']);
  check(hex(value.source.database_sha256, 64) && Number.isSafeInteger(value.source.schema)
    && value.source.schema >= 0 && value.source.schema <= 4294967295
    && Number.isSafeInteger(value.source.converted_at) && value.source.converted_at >= 0 && value.source.converted_at <= 8640000000000);
  let previous = after || 0;
  for (const item of value.items) {
    record(item); check(item.ordinal > previous); previous = item.ordinal;
  }
  if (value.next_cursor !== null) check(value.items.length > 0 && ordinal(value.next_cursor) && value.next_cursor === previous);
  return value;
}

export function nativeScalarText(value) {
  scalar(value);
  if (value.type === 'null') return 'NULL';
  if (value.type === 'real') return '0x' + value.value + ' (IEEE 754 bits)';
  if (value.type === 'blob' || value.type === 'text_bytes') return value.value.map(n => n.toString(16).padStart(2, '0')).join(' ');
  return value.value;
}
