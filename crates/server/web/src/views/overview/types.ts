// SPDX-License-Identifier: AGPL-3.0-or-later
export interface ActivityDay { day: number; create: number; append: number; revise: number; archive: number; restore: number }
export interface Writer { client_id: string | null; label: string; writes: number; last_write_at: number }
export interface Overview {
  workspace_id: string;
  generated_at: number;
  days: number;
  totals: {shelves: number; books: number; entries: number; archived_entries: number};
  activity: ActivityDay[];
  writers: Writer[];
  access: {active_clients: number; credentials_expiring_within_7d: number; pending_invitations: number};
}

/** Series in fixed categorical order; colours follow the series, never rank. */
export const SERIES = [
  {key: 'create', label: 'Created', fill: 'fill-chart-1', swatch: 'bg-chart-1', stroke: 'stroke-chart-1', value: (d: ActivityDay) => d.create},
  {key: 'append', label: 'Appended', fill: 'fill-chart-2', swatch: 'bg-chart-2', stroke: 'stroke-chart-2', value: (d: ActivityDay) => d.append},
  {key: 'revise', label: 'Revised', fill: 'fill-chart-3', swatch: 'bg-chart-3', stroke: 'stroke-chart-3', value: (d: ActivityDay) => d.revise},
  {key: 'lifecycle', label: 'Archived or restored', fill: 'fill-chart-4', swatch: 'bg-chart-4', stroke: 'stroke-chart-4', value: (d: ActivityDay) => d.archive + d.restore},
] as const;

export const total = (d: ActivityDay): number => d.create + d.append + d.revise + d.archive + d.restore;
export const compact = new Intl.NumberFormat(undefined, {notation: 'compact', maximumFractionDigits: 1});
export const whole = new Intl.NumberFormat();
/** Activity days are UTC calendar days. */
export const dayLabel = (day: number, long = false): string =>
  new Date(day * 1000).toLocaleDateString(undefined, long ? {weekday: 'short', month: 'short', day: 'numeric', timeZone: 'UTC'} : {month: 'short', day: 'numeric', timeZone: 'UTC'});

export interface AttentionItem { tone: 'warning' | 'info'; title: string; detail: string; view: import('../../lib/types').View }
