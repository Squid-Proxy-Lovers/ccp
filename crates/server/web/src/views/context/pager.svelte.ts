// SPDX-License-Identifier: AGPL-3.0-or-later
// One bounded, cursor-paged list (32 per page). Loads throw so callers can tell
// a failed refresh apart from the write that preceded it.
import {ReadTask} from '../../lib/readTask.js';
import type {ViewApp} from '../../lib/app.svelte';
import {PAGE_SIZE, type ContentPage} from './content';

export class Pager<T extends {id: string}> {
  items = $state.raw<T[]>([]);
  next = $state<string | null>(null);
  after = $state<string | null>(null);
  page = $state(1);
  loaded = $state(false);
  loading = $state(false);
  error = $state('');
  #read: ReadTask<ViewApp>;
  #fetch: (after: string | null, limit: number, app: ViewApp) => Promise<ContentPage<T>>;

  constructor(app: ViewApp, fetch: (after: string | null, limit: number, app: ViewApp) => Promise<ContentPage<T>>) { this.#fetch = fetch; this.#read = new ReadTask(app); }

  async load(after: string | null = null, page = 1): Promise<void> {
    await this.#read.run(readApp => this.#fetch(after, PAGE_SIZE, readApp), {
      start: () => { this.loading = true; this.error = ''; },
      receive: data => {
        this.items = data.items; this.next = data.next_cursor; this.after = after;
        this.page = page; this.loaded = true;
      },
      error: error => { this.error = (error as Error).message; },
      finish: () => { this.loading = false; }, rethrow: true,
    });
  }

  first(): Promise<void> { return this.load(); }
  forward(): Promise<void> { return this.next ? this.load(this.next, this.page + 1) : Promise.resolve(); }
  /** Re-read the page currently shown. */
  reload(): Promise<void> { return this.load(this.after, this.page); }
  clear(): void {
    this.#read.invalidate();
    this.items = []; this.next = null; this.after = null; this.page = 1;
    this.loaded = false; this.loading = false; this.error = '';
  }
}
