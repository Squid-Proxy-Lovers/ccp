// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared by the installed-dependency suite and isolated before/after validation.
import assert from 'node:assert/strict';

const payload = () => JSON.parse('{"__proto__":{"injected":true}}');
function clean(value, prototype) {
  assert.equal(Object.getPrototypeOf(value), prototype);
  assert.equal(value.injected, undefined);
  assert.equal(Object.hasOwn(value, 'injected'), false);
}
const pluginMerge = (merge, ...configs) => configs.reduce((target, source) => merge(target, source, {
  arrayMerge: (previous, next) => next ?? previous,
}), {});

export const mergeCases = {
  ordinary(merge) {
    const source = {compilerOptions: {dev: false}};
    const result = pluginMerge(merge, {compilerOptions: {css: 'external'}, extensions: ['.svelte']}, source);
    assert.equal(JSON.stringify(result), '{"compilerOptions":{"css":"external","dev":false},"extensions":[".svelte"]}');
    assert.notEqual(result.compilerOptions, source.compilerOptions);
    return JSON.stringify(result);
  },
  nested_placeholder(merge, prototype) {
    // Reaches the plugin's actual reduce-from-{} wrapper, not just direct merge.
    for (const placeholder of [null, undefined, 0, false, '']) {
      const result = pluginMerge(merge, {session: placeholder}, {session: payload()});
      clean(result.session, prototype);
    }
  },
  null_prototype_target(merge, prototype) {
    clean(merge(Object.create(null), payload()), prototype);
  },
  target_copy(merge, prototype) {
    clean(merge(payload(), {safe: 1}), prototype);
  },
  array_clone(merge, prototype) {
    const result = merge([], [payload()]);
    assert.equal(result.length, 1);
    clean(result[0], prototype);
  },
  symbols_instances_and_specials(merge, prototype) {
    const key = Symbol('metadata');
    class Config { constructor() { this.enabled = true; } }
    const date = new Date(0), pattern = /svelte/u;
    const input = {compilerOptions: {[key]: {add: 2}}, config: new Config(), date, pattern};
    const result = pluginMerge(merge, {compilerOptions: {[key]: {keep: 1}}}, input);
    assert.equal(JSON.stringify(result.compilerOptions[key]), '{"keep":1,"add":2}');
    assert.notEqual(result.compilerOptions[key], input.compilerOptions[key]);
    assert.equal(Object.getPrototypeOf(result.config), prototype);
    assert.equal(result.config.enabled, true);
    assert.equal(result.date, date);
    assert.equal(result.pattern, pattern);
  },
  adjacent_keys(merge, prototype) {
    const result = merge({}, JSON.parse('{"constructor":{"prototype":{"injected":true}},"prototype":{"safe":1},"safe":2}'));
    clean(result, prototype);
    assert.equal(result.constructor, prototype.constructor);
    assert.equal(result.safe, 2);
    assert.equal(result.prototype.safe, 1);
    assert.equal(({}).injected, undefined);
    assert.equal(prototype.injected, undefined);
  },
  options_compatibility(merge) {
    const extensions = ['.custom'];
    const result = pluginMerge(merge, {extensions: ['.svelte'], compilerOptions: {dev: true}},
      {extensions, compilerOptions: {css: 'external'}});
    assert.equal(result.extensions, extensions);
    assert.equal(JSON.stringify(result), '{"extensions":[".custom"],"compilerOptions":{"dev":true,"css":"external"}}');
    const combined = merge.all([{one: 1}, {two: 2}]);
    assert.equal(JSON.stringify(combined), '{"one":1,"two":2}');
  },
};
