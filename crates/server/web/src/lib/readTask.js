// SPDX-License-Identifier: AGPL-3.0-or-later
// A read owns its receive/error/loading effects, independently of view lifetime.
export class SupersededRead extends Error {
  constructor() { super('Read superseded.'); this.name = 'SupersededRead'; }
}
export const isSupersededRead = error => error?.name === 'SupersededRead';

/** @template {{generation: number, session: unknown, sessionEpoch?: number, effect: (fn: () => void) => void, isCurrent?: () => boolean, read?: Function}} A */
export class ReadTask {
  #serial = 0;
  /** @param {A} app */
  constructor(app) { this.app = app; }

  /**
   * @template T
   * @param {(app: A) => Promise<T>} load
   * @param {{receive: (data: T) => void, error?: (error: unknown) => void, start?: () => void, finish?: () => void, rethrow?: boolean}} effects
   * @returns {Promise<T | undefined>}
   */
  async run(load, effects) {
    const serial = ++this.#serial, generation = this.app.generation;
    const session = this.app.session, epoch = this.app.sessionEpoch;
    const current = () => serial === this.#serial && generation === this.app.generation
      && session === this.app.session && epoch === this.app.sessionEpoch
      && (this.app.isCurrent?.() ?? true);
    const commit = fn => { if (current() && fn) this.app.effect(fn); };
    commit(effects.start);
    try {
      // The loader must use this bound API, including after its own awaits.
      // Each request carries this exact ticket to the session-effect boundary.
      const scoped = new Proxy(this.app, {get: (target, key) => key === 'read'
        ? (path, body) => target.read(path, body, current)
        : Reflect.get(target, key, target)});
      const data = await load(scoped);
      if (!current()) return;
      let received = false;
      commit(() => { effects.receive(data); received = true; });
      return received ? data : undefined;
    } catch (error) {
      if (!current() || isSupersededRead(error)) return;
      commit(() => effects.error?.(error));
      if (effects.rethrow) throw error;
    } finally { commit(effects.finish); }
  }

  invalidate() { this.#serial += 1; }
}
