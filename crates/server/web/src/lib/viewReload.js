// SPDX-License-Identifier: AGPL-3.0-or-later
// Registrations belong to a mounted view/session. Cleanup removes only its own
// entry, so an old destructor cannot remove a replacement view's loader.
export class ViewReloads {
  #entries = new Map();
  register(state, reload) {
    const key = {}, session = state.session, epoch = state.sessionEpoch;
    this.#entries.set(key, {session, epoch, reload});
    return () => { this.#entries.delete(key); };
  }
  has(state) {
    return [...this.#entries.values()].some(entry => entry.session === state.session && entry.epoch === state.sessionEpoch);
  }
  async run(state) {
    const session = state.session, epoch = state.sessionEpoch, generation = state.generation;
    let failure;
    for (const [key, entry] of [...this.#entries]) {
      if (state.session !== session || state.sessionEpoch !== epoch || state.generation !== generation || state.locked) return;
      if (this.#entries.has(key) && entry.session === session && entry.epoch === epoch) {
        try { await entry.reload(); } catch (error) { failure ??= error; }
      }
    }
    if (state.session !== session || state.sessionEpoch !== epoch || state.generation !== generation || state.locked) return;
    if (failure) throw failure;
  }
}
