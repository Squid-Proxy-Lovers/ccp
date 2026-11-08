// SPDX-License-Identifier: AGPL-3.0-or-later
import {seconds} from '../../lib/api';
import type {Activity} from '../../lib/types';

/** One chart column: writes that started at `day` (Unix seconds) for one bucket. */
export interface ActivityDay { day: number; create: number; append: number }
export type Unit = 'hour' | 'day';

/** Series in fixed categorical order; colours follow the series, never rank. */
export const SERIES = [
  {key: 'create', label: 'Created', fill: 'fill-chart-1', swatch: 'bg-chart-1', stroke: 'stroke-chart-1', value: (d: ActivityDay) => d.create},
  {key: 'append', label: 'Appended', fill: 'fill-chart-2', swatch: 'bg-chart-2', stroke: 'stroke-chart-2', value: (d: ActivityDay) => d.append},
] as const;

export const total = (d: ActivityDay): number => d.create + d.append;
export const compact = new Intl.NumberFormat(undefined, {notation: 'compact', maximumFractionDigits: 1});
export const whole = new Intl.NumberFormat();

export function bucketLabel(at: number, unit: Unit, long = false): string {
  const date = new Date(at * 1000);
  if (unit === 'hour') {
    return long
      ? date.toLocaleString(undefined, {weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit'})
      : date.toLocaleTimeString(undefined, {hour: 'numeric'});
  }
  return date.toLocaleDateString(undefined, long ? {weekday: 'short', month: 'short', day: 'numeric'} : {month: 'short', day: 'numeric'});
}

/**
 * Bucket the newest writes the server returned. The API caps a read at 500
 * events, so the chart covers exactly the span those events reach back to:
 * hourly columns for up to two days, daily columns (max 30) beyond that.
 */
export function bucket(events: Activity[], now = Date.now() / 1000): {days: ActivityDay[]; unit: Unit; from: number} {
  const stamps = events.map(event => seconds(event.created_at));
  const times = stamps.filter(value => value > 0);
  const oldest = times.length ? Math.min(...times) : now - 24 * 3600;
  const span = Math.max(3600, now - oldest);
  const unit: Unit = span <= 48 * 3600 ? 'hour' : 'day';
  const size = unit === 'hour' ? 3600 : 86400;
  const count = Math.min(unit === 'hour' ? 48 : 30, Math.max(unit === 'hour' ? 12 : 7, Math.ceil(span / size)));
  const start = (at: number) => {
    if (unit === 'hour') return Math.floor(at / 3600) * 3600;
    const date = new Date(at * 1000);
    date.setHours(0, 0, 0, 0);
    return date.getTime() / 1000;
  };
  const last = start(now);
  const days: ActivityDay[] = [];
  for (let index = count - 1; index >= 0; index--) {
    const day = unit === 'hour' ? last - index * 3600 : start(last - index * 86400 + 43200);
    days.push({day, create: 0, append: 0});
  }
  const lookup = new Map(days.map(item => [item.day, item]));
  events.forEach((event, index) => {
    const slot = lookup.get(start(stamps[index] ?? 0));
    if (!slot) return;
    if (event.kind === 'entry_appended') slot.append += 1;
    else slot.create += 1;
  });
  return {days, unit, from: days[0]?.day ?? oldest};
}
