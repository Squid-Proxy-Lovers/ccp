// SPDX-License-Identifier: AGPL-3.0-or-later
// Toast notices in one polite live region. Messages never contain secrets.
export type Tone = 'info' | 'success' | 'warning' | 'destructive';
export interface Toast { id: number; text: string; tone: Tone }
export const toasts = $state<Toast[]>([]);
let serial = 0;

export function notice(text: string, tone: Tone = 'info'): void {
  if (!text) return;
  const id = ++serial;
  toasts.push({id, text, tone});
  if (toasts.length > 4) toasts.shift();
  // Errors and warnings stay until dismissed; they often carry recovery steps.
  if (tone === 'info' || tone === 'success') setTimeout(() => dismiss(id), 6000);
}

export function dismiss(id: number): void {
  const index = toasts.findIndex(toast => toast.id === id);
  if (index >= 0) toasts.splice(index, 1);
}

export function clearToasts(): void { toasts.splice(0); }

export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    notice('Copied. Keep it private.', 'success');
    return true;
  } catch {
    notice('Clipboard unavailable. Select and copy the text manually.', 'warning');
    return false;
  }
}

export function stamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(undefined, {dateStyle: 'medium', timeStyle: 'short'});
}

export function relative(seconds: number, now = Date.now()): string {
  const delta = Math.round((seconds * 1000 - now) / 1000);
  const format = new Intl.RelativeTimeFormat(undefined, {numeric: 'auto'});
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [['year', 31536000], ['month', 2592000], ['week', 604800], ['day', 86400], ['hour', 3600], ['minute', 60]];
  for (const [unit, size] of steps) if (Math.abs(delta) >= size) return format.format(Math.round(delta / size), unit);
  return format.format(delta, 'second');
}
