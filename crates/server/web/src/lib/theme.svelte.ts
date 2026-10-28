// SPDX-License-Identifier: AGPL-3.0-or-later
// Appearance preferences: mode, base tint and accent. These are the only values
// this panel stores in the browser; public/theme.js applies them before first paint.
export type ThemeMode = 'light' | 'dark' | 'system';
export const BASES = ['neutral', 'zinc', 'slate', 'stone'] as const;
export const ACCENTS = ['neutral', 'blue', 'violet', 'green', 'orange', 'rose'] as const;
export type Base = (typeof BASES)[number];
export type Accent = (typeof ACCENTS)[number];

const query = matchMedia('(prefers-color-scheme: dark)');

function read<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const saved = localStorage.getItem(key);
    return allowed.includes(saved as T) ? (saved as T) : fallback;
  } catch { return fallback; }
}
function store(key: string, value: string, fallback: string): void {
  try {
    if (value === fallback) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch { /* storage unavailable: preference lasts for this page only */ }
}

class Theme {
  mode = $state<ThemeMode>(read('ccp-theme', ['light', 'dark'] as ThemeMode[], 'system'));
  base = $state<Base>(read('ccp-base', BASES, 'neutral'));
  accent = $state<Accent>(read('ccp-accent', ACCENTS, 'neutral'));
  systemDark = $state(query.matches);
  dark = $derived(this.mode === 'dark' || (this.mode === 'system' && this.systemDark));

  constructor() {
    query.addEventListener('change', event => { this.systemDark = event.matches; this.#apply(); });
    this.#apply();
  }
  set(mode: ThemeMode): void { this.mode = mode; store('ccp-theme', mode, 'system'); this.#apply(); }
  setBase(base: Base): void { this.base = base; store('ccp-base', base, 'neutral'); this.#apply(); }
  setAccent(accent: Accent): void { this.accent = accent; store('ccp-accent', accent, 'neutral'); this.#apply(); }
  reset(): void { this.set('system'); this.setBase('neutral'); this.setAccent('neutral'); }

  #apply(): void {
    const root = document.documentElement;
    // Swap every colour in one frame rather than animating hover/border transitions.
    root.classList.add('theme-switching');
    root.classList.toggle('dark', this.dark);
    if (this.base === 'neutral') delete root.dataset.base; else root.dataset.base = this.base;
    if (this.accent === 'neutral') delete root.dataset.accent; else root.dataset.accent = this.accent;
    requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('theme-switching')));
  }
}
export const theme = new Theme();
