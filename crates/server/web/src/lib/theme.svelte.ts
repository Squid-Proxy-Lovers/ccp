// SPDX-License-Identifier: AGPL-3.0-or-later
// Colour theme preference. The only value this panel stores in the browser.
export type ThemeMode = 'light' | 'dark' | 'system';
const query = matchMedia('(prefers-color-scheme: dark)');

function read(): ThemeMode {
  try {
    const saved = localStorage.getItem('ccp-theme');
    return saved === 'light' || saved === 'dark' ? saved : 'system';
  } catch { return 'system'; }
}

class Theme {
  mode = $state<ThemeMode>(read());
  systemDark = $state(query.matches);
  dark = $derived(this.mode === 'dark' || (this.mode === 'system' && this.systemDark));
  constructor() {
    query.addEventListener('change', event => { this.systemDark = event.matches; this.#apply(); });
    this.#apply();
  }
  set(mode: ThemeMode): void {
    this.mode = mode;
    try {
      if (mode === 'system') localStorage.removeItem('ccp-theme');
      else localStorage.setItem('ccp-theme', mode);
    } catch { /* storage unavailable: preference lasts for this page only */ }
    this.#apply();
  }
  #apply(): void { document.documentElement.classList.toggle('dark', this.dark); }
}
export const theme = new Theme();
