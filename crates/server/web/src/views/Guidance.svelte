<script lang="ts">
  // Instruction boards. Global guidance needs an instance key; a workspace board
  // needs this active workspace's owner or admin key. Global guidance is
  // read-only inside a workspace view.
  import {onMount, tick} from 'svelte';
  import {RefreshCw} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import SkeletonCard from '../lib/ui/skeleton/SkeletonCard.svelte';
  import Board from './guidance/Board.svelte';
  import {app} from '../lib/app.svelte';
  import {notice} from '../lib/notify.svelte';
  import {canPublish} from '../lib/model.js';
  import {readGuidance, type Board as BoardData, type Bundle} from './guidance/guidance';

  let {global = false}: {global?: boolean} = $props();

  // svelte-ignore state_referenced_locally
  const workspaceId = global ? null : (app.workspace?.id ?? null);
  let bundle = $state.raw<Bundle | null>(null);
  let failed = $state('');

  const boards = $derived(bundle ? [bundle.global, bundle.workspace].filter((board): board is BoardData => board !== null) : []);

  async function load(): Promise<void> {
    try {
      bundle = await readGuidance(workspaceId);
      failed = '';
    } catch (error) {
      if (app.locked) return;
      failed = (error as Error).message;
      notice(failed, 'destructive');
    }
  }
  onMount(() => { void load(); });

  function editable(board: BoardData): boolean {
    if (!app.session || (board.scope.kind === 'global' && workspaceId)) return false;
    return canPublish(app.session, board.scope, app.workspace);
  }

  function note(board: BoardData): string {
    const global = board.scope.kind === 'global';
    if (editable(board)) return 'Your key can save this board. Saving goes live immediately.';
    if (global && workspaceId) return 'Read-only here. Edit instance-wide guidance under Global guidance.';
    if (global) return 'Read-only with a workspace key. Saving global guidance needs an instance key.';
    return 'Read-only. Saving needs this workspace’s owner or admin key while the workspace is active.';
  }
  // Replacing the bundle remounts the boards; return focus to the board just saved or reloaded.
  async function replace(next: Bundle, kind: string) {
    bundle = next;
    await tick();
    document.getElementById(`guidance-${kind}`)?.focus();
  }
</script>

<div class="space-y-6">
  <PageHeader title={global ? 'Global guidance' : 'Guidance'}
    description="Global guidance takes precedence; workspace guidance adds compatible detail. Host instructions and actual server permissions stay authoritative." />

  <Alert title="Saves go live immediately">
    <p>Agents receive new or changed guidance with applicable CCP tool responses. That doesn't prove they read or followed it. This page shows a loaded snapshot.</p>
  </Alert>

  {#if failed}
    <Alert tone="destructive" title="Guidance unavailable" live>
      {#snippet action()}
        <Button size="sm" variant="outline" busy={app.working('guidance-load')} onclick={() => app.run(load, 'guidance-load')}><RefreshCw /> Try again</Button>
      {/snippet}
      <p>{failed}</p>
    </Alert>
  {:else if !bundle}
    <Delayed>
      <div class={['grid gap-6', !global && 'xl:grid-cols-2']}>
        {#each global ? [0] : [0, 1] as index (index)}
          <SkeletonCard action label={index === 0 ? 'Loading guidance' : undefined}>
            <div class="space-y-3 px-5">
              <Skeleton class="h-64 w-full" />
              <div class="flex items-center justify-between"><Skeleton class="h-3 w-28 opacity-70" /><Skeleton class="h-9 w-32" /></div>
            </div>
          </SkeletonCard>
        {/each}
      </div>
    </Delayed>
  {:else}
    {#key bundle}
      <div class={['grid items-start gap-6', boards.length > 1 && 'xl:grid-cols-2']}>
        {#each boards as board (board.scope.kind)}
          <Board {board} {workspaceId} editable={editable(board)} note={note(board)} onreplace={next => replace(next, board.scope.kind)} />
        {/each}
      </div>
    {/key}
  {/if}
</div>
