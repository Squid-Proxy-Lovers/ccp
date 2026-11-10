// SPDX-License-Identifier: AGPL-3.0-or-later
// A read owns its receive/error/loading effects, independently of view lifetime.
export class SupersededRead extends Error {
  constructor() { super('Read superseded.'); this.name = 'SupersededRead'; }
}
export const isSupersededRead = error => error?.name === 'SupersededRead';

export class ReadTask {
  #serial = 0;
  /** @param {{generation: number, session: unknown, sessionEpoch?: number, effect: (fn: () => void) => void}} app */
  constructor(app) { this.app = app; }

  /**
   * @template T
   * @param {() => Promise<T>} load
   * @param {{receive: (data: T) => void, error?: (error: unknown) => void, start?: () => void, finish?: () => void, rethrow?: boolean}} effects
   * @returns {Promise<T | undefined>}
   */
  async run(load, effects) {
    const serial = ++this.#serial, generation = this.app.generation;
    const session = this.app.session, epoch = this.app.sessionEpoch;
    const current = () => serial === this.#serial && generation === this.app.generation
      && session === this.app.session && epoch === this.app.sessionEpoch;
    const commit = fn => { if (current() && fn) this.app.effect(fn); };
    commit(effects.start);
    try {
      const data = await load();
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
