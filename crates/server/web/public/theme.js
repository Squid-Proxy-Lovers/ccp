// SPDX-License-Identifier: AGPL-3.0-or-later
// Applies the saved colour theme before first paint. Stores no credentials.
(function () {
  var saved = null;
  try { saved = localStorage.getItem('ccp-theme'); } catch (_) {}
  var dark = saved === 'dark' || (saved !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.toggle('dark', dark);
})();
