// SPDX-License-Identifier: AGPL-3.0-or-later
// Guidance boards: shared types and the verified read used by the view and its editors.
import {guidanceBundle} from '../../lib/model.js';
import {app} from '../../lib/app.svelte';

export type GuidanceScope = {kind: 'global'} | {kind: 'workspace'; workspace_id: string};
export interface Publisher { recorded_at: number; actor_kind: string; actor_id: string; principal_id: string }
export interface Board { scope: GuidanceScope; revision: number; text: string; publisher: Publisher | null }
export interface Bundle { precedence: 'global_first'; global: Board; workspace: Board | null }

export const MAX_BYTES = 8192;
export const bytes = (text: string): number => new TextEncoder().encode(text).length;

export async function readGuidance(workspaceId: string | null): Promise<Bundle> {
  return guidanceBundle(await app.request('/api/v1/guidance/get', {workspace_id: workspaceId}), workspaceId) as Bundle;
}
