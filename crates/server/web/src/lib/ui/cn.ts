// SPDX-License-Identifier: AGPL-3.0-or-later
import type {ClassValue as SvelteClass} from 'svelte/elements';

export type ClassValue = SvelteClass | boolean | null | undefined;
// Svelte resolves nested class arrays and drops falsy entries; this only groups them.
export const cn = (...values: ClassValue[]): SvelteClass => values as SvelteClass;
