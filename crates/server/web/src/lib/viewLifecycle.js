// SPDX-License-Identifier: AGPL-3.0-or-later
// Register component lifetime once; operation scopes may capture the current
// session later (for persistent components such as deletion tracking).
import {onDestroy} from 'svelte';

/**
 * @template S
 * @param {{forView: (isMounted: () => boolean) => S}} app
 * @returns {() => S}
 */
export function viewLifetime(app) {
  let mounted = true;
  onDestroy(() => { mounted = false; });
  return () => app.forView(() => mounted);
}

/**
 * @template {{dispose: () => void}} S
 * @param {{forView: (isMounted?: () => boolean) => S}} app
 * @returns {S}
 */
export function mountedView(app) {
  const scope = app.forView();
  onDestroy(scope.dispose);
  return scope;
}
