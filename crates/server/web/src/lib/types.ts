// SPDX-License-Identifier: AGPL-3.0-or-later
export interface Scope { kind: 'instance' | 'workspace'; workspace_id?: string; role?: string }
export interface Session {
  instance_id: string;
  csrf_token: string;
  expires_at: number;
  identity: { key_id: string; key_name: string; principal_id: string; principal_name: string; scope: Scope };
}
export interface Workspace { id: string; name: string; archived: boolean; [key: string]: unknown }
/** Any frozen intent instance; identity matters, so keep the object rather than copying it. */
export interface PendingWork { readonly kind?: string; readonly body: string }
export type View = 'overview' | 'context' | 'access' | 'reported' | 'guidance' | 'settings' | 'global-guidance' | 'workspaces';
export interface Page<T> { items: T[]; next_cursor: string | null }
