// SPDX-License-Identifier: AGPL-3.0-or-later
// One ownership boundary for view callbacks, notifications and global mutations.
// A retired view may finish its local promises, but cannot affect a new visit.
/**
 * @template {{session: unknown, generation: number}} T
 * @template {Record<string, Function>} E
 * @param {T} state
 * @param {E} effects
 * @returns {T & E}
 */
export function viewScope(state, effects) {
  const session = state.session;
  const generation = state.generation;
  const current = () => state.session === session && state.generation === generation;
  const retired = () => new Error('This view has been superseded.');
  return new Proxy(state, {
    get(target, key) {
      if (key in effects) {
        return (...args) => {
          if (!current()) {
            if (key === 'confirmAction') return Promise.resolve(false);
            if (key === 'wire') return Promise.reject(retired());
            return undefined;
          }
          if (key === 'copyText') {
            return effects.copyText(...args, (...message) => { if (current()) effects.notice(...message); });
          }
          const result = effects[key](...args);
          if (key === 'wire') return result.then(value => { if (!current()) throw retired(); return value; });
          if (key === 'confirmAction') return result.then(ok => ok && current());
          return result;
        };
      }
      const value = Reflect.get(target, key, target);
      if (typeof value !== 'function') return value;
      return (...args) => {
        if (!current()) {
          if (key === 'request' || key === 'verify') return Promise.reject(retired());
          if (key === 'clearWorkspace') throw retired();
          return undefined;
        }
        const result = value.apply(target, args);
        return result;
      };
    },
    set(target, key, value) {
      if (!current()) return true;
      const changed = Reflect.set(target, key, value, target);
      return changed;
    },
  });
}
