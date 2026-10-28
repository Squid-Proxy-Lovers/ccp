// SPDX-License-Identifier: AGPL-3.0-or-later
// One bounded, cursor-paged list (32 per page). Loads throw so callers can tell
// a failed refresh apart from the write that preceded it.
import {PAGE_SIZE, type ContentPage} from './content';

export class Pager<T extends {id: string}> {
  items = $state.raw<T[]>([]);
  next = $state<string | null>(null);
  after = $state<string | null>(null);
  page = $state(1);
  loaded = $state(false);
  loading = $state(false);
  error = $state('');
  #serial = 0;
  #fetch: (after: string | null, limit: number) => Promise<ContentPage<T>>;

  constructor(fetch: (after: string | null, limit: number) => Promise<ContentPage<T>>) { this.#fetch = fetch; }

  async load(after: string | null = null, page = 1): Promise<void> {
    const serial = ++this.#serial;
    this.loading = true;
    this.error = '';
    try {
      const data = await this.#fetch(after, PAGE_SIZE);
      if (serial !== this.#serial) return;
      this.items = data.items;
      this.next = data.next_cursor;
      this.after = after;
      this.page = page;
      this.loaded = true;
    } catch (error) {
      if (serial === this.#serial) this.error = (error as Error).message;
      throw error;
    } finally {
      if (serial === this.#serial) this.loading = false;
    }
  }

  first(): Promise<void> { return this.load(); }
  forward(): Promise<void> { return this.next ? this.load(this.next, this.page + 1) : Promise.resolve(); }
  /** Re-read the page currently shown. */
  reload(): Promise<void> { return this.load(this.after, this.page); }
  clear(): void {
    this.#serial += 1;
    this.items = []; this.next = null; this.after = null; this.page = 1;
    this.loaded = false; this.loading = false; this.error = '';
  }
}
