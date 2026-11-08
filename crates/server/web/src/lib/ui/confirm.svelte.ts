// SPDX-License-Identifier: AGPL-3.0-or-later
// Promise-based confirmation rendered by ConfirmHost, replacing window.confirm.
export interface ConfirmRequest {
  title: string;
  body: string;
  confirm: string;
  destructive?: boolean;
  resolve: (ok: boolean) => void;
}
export const confirmState = $state<{current: ConfirmRequest | null}>({current: null});
export function confirmAction(options: Omit<ConfirmRequest, 'resolve'>): Promise<boolean> {
  confirmState.current?.resolve(false);
  return new Promise(resolve => { confirmState.current = {...options, resolve}; });
}
