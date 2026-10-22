// SPDX-License-Identifier: AGPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source = await readFile(new URL('../crates/server/src/workspace_http/panel/native_values.js', import.meta.url), 'utf8');
const {nativeHistoryPage, nativeScalarText} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const page = () => ({workspace_id: 'workspace', entry_id: 'entry', source: {database_sha256: 'a'.repeat(64), schema: 2, converted_at: 1}, items: [{ordinal: 1, source_table: 'message_history', source_key: '-42', row: {columns: ['id', 'text', 'real', 'bytes', 'blob', 'null'], values: [
  {type: 'integer', value: '9223372036854775807'}, {type: 'text', value: '<img src=x onerror="danger()">\u0000literal'},
  {type: 'real', value: '8000000000000000'}, {type: 'text_bytes', value: [0, 255]}, {type: 'blob', value: [0, 255]}, {type: 'null'},
]}}], next_cursor: 1});
const read = (value, after = null) => nativeHistoryPage(value, 'workspace', 'entry', after);

test('source pages preserve typed data and allow byte-budget partial count pages', () => {
  const value = page(), before = JSON.stringify(value);
  assert.equal(read(value), value);
  assert.equal(JSON.stringify(value), before);
  assert.equal(nativeScalarText(value.items[0].row.values[0]), '9223372036854775807');
  assert.equal(nativeScalarText(value.items[0].row.values[1]), '<img src=x onerror="danger()">\u0000literal');
  assert.equal(nativeScalarText(value.items[0].row.values[2]), '0x8000000000000000 (IEEE 754 bits)');
  assert.equal(nativeScalarText(value.items[0].row.values[3]), '00 ff');
  assert.equal(nativeScalarText(value.items[0].row.values[4]), '00 ff');
  assert.equal(nativeScalarText(value.items[0].row.values[5]), 'NULL');
  value.items = []; value.next_cursor = null; read(value);
  value.source = null; read(value);
});

test('source scopes headers keys and ordinal cursors reject mismatches without echoing bodies', () => {
  for (const alter of [v => {v.workspace_id = 'foreign';}, v => {v.entry_id = 'foreign';},
    v => {v.source.database_sha256 = 'A'.repeat(64);}, v => {v.source.recorded_by = 'PRIVATE_BODY';},
    v => {v.source.converted_at = 8640000000001;}, v => {v.source.schema = 4294967296;},
    v => {v.source = null;}, v => {v.items[0].source_table = 'auth_tokens';},
    v => {v.items[0].source_key = '-0';}, v => {v.items[0].ordinal = 0;},
    v => {v.items[0].ordinal = 9007199254740992;}, v => {v.next_cursor = 2;},
    v => {v.items.push(structuredClone(v.items[0]));},
  ]) {
    const value = page(); alter(value);
    assert.throws(() => read(value), error => !error.message.includes('PRIVATE_BODY') && /CLI/.test(error.message));
  }
  assert.throws(() => read(page(), 1)); assert.throws(() => read(page(), '0'));
  const value = page(); value.items[0].source_table = 'deleted_message_packs'; value.items[0].source_key = 'archive\u0000key'; read(value, 0);
  value.items[0].source_key = ''; assert.throws(() => read(value));
});

test('source scalar classes column shapes and encoded-row bounds are explicit', () => {
  for (const scalar of [{type: 'integer', value: 1}, {type: 'integer', value: '01'}, {type: 'integer', value: '-0'},
    {type: 'integer', value: '9223372036854775808'}, {type: 'real', value: 'not-hex'}, {type: 'real', value: 'ABC0000000000000'},
    {type: 'text', value: 1}, {type: 'blob', value: [true]}, {type: 'text_bytes', value: [256]}, {type: 'null', value: 'extra'}]) {
    assert.throws(() => nativeScalarText(scalar));
  }
  const value = page(); value.items[0].row.columns[1] = 'id'; assert.throws(() => read(value));
  const long = page(); long.items[0].row.values[1].value = 'x'.repeat(2 * 1024 * 1024); assert.throws(() => read(long));
  const bytes = Array.from({length: 256}, (_, n) => n);
  assert.equal(nativeScalarText({type: 'blob', value: bytes}).split(' ').length, 256);
  assert.equal(nativeScalarText({type: 'blob', value: bytes}).endsWith('fe ff'), true);
});
