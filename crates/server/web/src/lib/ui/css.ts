// SPDX-License-Identifier: AGPL-3.0-or-later
// Sets computed geometry through the CSSOM. The CSP forbids style attributes,
// but property writes on element.style are allowed.
import type {Action} from 'svelte/action';

export const css: Action<HTMLElement, Partial<Record<'width' | 'height' | 'left' | 'top' | 'transform', string>>> = (node, styles) => {
  const apply = (next: typeof styles) => { for (const [name, value] of Object.entries(next ?? {})) node.style.setProperty(name, value ?? ''); };
  apply(styles);
  return {update: apply};
};
