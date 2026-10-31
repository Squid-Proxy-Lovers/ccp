// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared content types and helpers for the context browser.
import {tick} from 'svelte';
import type {Page} from '../../lib/types';

export const PAGE_SIZE = 32;
export const contentPath = (name: string): string => '/api/v1/content/' + name;

export interface Shelf { id: string; workspace_id: string; name: string; description: string; created_at: number }
export interface Book extends Shelf { shelf_id: string }
export interface EntrySummary {
  id: string; workspace_id: string; book_id: string; name: string; owner_client_id: string | null;
  revision: number; archived: boolean; created_at: number; updated_at: number;
}
export interface EntryContent { text: string; description: string; labels: string[] }
export interface EntryRevision {
  revision: number; action: string; actor: {kind: string; id: string}; client_id: string | null;
  admin_principal_id: string | null; agent: string | null; recorded_at: number;
}
export interface Snapshot { entry: EntrySummary; revision: EntryRevision; content: EntryContent; appended_text: string | null }
export interface EntryReceipt { entry_id: string; revision: number }
export interface HistoryPage { items: EntryRevision[]; next_cursor: number | null }
export interface ShelfStateValue { workspace_id: string; shelf_id: string; archived: boolean; revision: number }
export interface ArchivedShelf { id: string; workspace_id: string; name: string; revision: number }
export type ContentPage<T> = Page<T>;

/** An exact retryable write held in `app.pending` until confirmed or explicitly forgotten. */
export interface HeldIntent { body: string; request(session: unknown): {body: string; proof: string} }

/** A lost or unverifiable response: the request may have completed. */
export function outcomeUnknown(error: unknown): boolean {
  const status = (error as {status?: number}).status;
  return status === undefined || status >= 500;
}

export const message = (error: unknown): string => (error as Error).message;

/** Move focus after the DOM settles; used for deliberate focus handoffs. */
export async function focusId(id: string): Promise<void> {
  await tick();
  document.getElementById(id)?.focus();
}

export async function focusRow(key: string): Promise<void> {
  await tick();
  document.querySelector<HTMLElement>(`[data-row="${CSS.escape(key)}"]`)?.focus();
}

/** The three panes sit side by side from the md breakpoint up. */
export const wide = (): boolean => window.matchMedia('(min-width: 48rem)').matches;

export const actorText = (revision: EntryRevision): string => `${revision.actor.kind.replace('_', ' ')} ${revision.actor.id}`;
