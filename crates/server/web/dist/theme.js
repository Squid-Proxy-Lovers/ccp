// SPDX-License-Identifier: AGPL-3.0-or-later
// Applies the saved appearance before first paint. Stores no credentials.
(function () {
  var root = document.documentElement;
  function read(key, allowed) {
    try { var value = localStorage.getItem(key); return allowed.indexOf(value) >= 0 ? value : null; } catch (_) { return null; }
  }
  var mode = read('ccp-theme', ['light', 'dark']);
  root.classList.toggle('dark', mode === 'dark' || (mode !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches));
  var base = read('ccp-base', ['zinc', 'slate', 'stone']);
  if (base) root.setAttribute('data-base', base);
  var accent = read('ccp-accent', ['blue', 'violet', 'green', 'orange', 'rose']);
  if (accent) root.setAttribute('data-accent', accent);
})();
