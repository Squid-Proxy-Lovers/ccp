// SPDX-License-Identifier: AGPL-3.0-or-later
// One ownership boundary for view callbacks, notifications and global mutations.
// Read generations do not own views. Only session replacement or component
// destruction retires their effects; local promises still settle normally.
/**
 * @template {{session: unknown, sessionEpoch?: number}} T
 * @template {Record<string, Function>} E
 * @param {T} state
 * @param {E} effects
 * @param {() => boolean} [isMounted]
 * @returns {T & E & {dispose: () => void}}
 */
export function viewScope(state, effects, isMounted = () => true) {
  const session = state.session;
  const epoch = state.sessionEpoch ?? 0;
  let active = true;
  const current = () => active && isMounted() && state.session === session && (state.sessionEpoch ?? 0) === epoch;
  const retired = () => new Error('This view has been superseded.');
  return new Proxy(state, {
    get(target, key) {
      if (key === 'dispose') return () => { active = false; };
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
        if (key === 'run') {
          const [action, ...rest] = args;
          return value.call(target, async () => {
            try { return await action(); }
            catch (error) { if (current()) throw error; }
          }, ...rest);
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
