<script lang="ts">
  // Body-free shelf lifecycle observation with owner/admin archive or restore.
  import {onMount} from 'svelte';
  import {Archive, ArchiveRestore, RotateCcw} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {ContentAdminIntent, canManageInvitations, shelfState} from '../../lib/model.js';
  import {contentPath, message, type ShelfStateValue} from './content';
  import {sendArchive, type AdminIntent, type ArchiveOutcome} from './archiveWrite';

  let {shelfId, onarchive, onbrowse}: {
    shelfId: string; onarchive: (intent: AdminIntent, outcome: ArchiveOutcome) => void; onbrowse: () => void;
  } = $props();

  const workspace = app.workspace;
  const scoped = Boolean(app.session && canManageInvitations(app.session, workspace));
  let observed = $state.raw<ShelfStateValue | null>(null);
  let error = $state('');

  async function load() {
    error = '';
    try {
      observed = shelfState(await app.request(contentPath('shelves/state'), {workspace_id: workspace?.id, node_id: shelfId}), workspace?.id, shelfId);
    } catch (failure) { if (!app.locked) error = message(failure); }
  }

  function transition() {
    const current = observed;
    if (!current || !workspace) return;
    void app.run(async () => {
      if (!app.mayLeave()) return;
      const verb = current.archived ? 'Restore' : 'Archive';
      const ok = await confirmAction({
        title: `${verb} this shelf?`,
        body: `${verb} shelf ${shelfId} in workspace ${workspace.id} at lifecycle revision ${current.revision}? Descendant IDs and individual archive state are preserved.`,
        confirm: `${verb} shelf`, destructive: !current.archived,
      });
      if (!ok) return;
      const intent = new ContentAdminIntent(app.session, workspace, current, 'shelf') as unknown as AdminIntent;
      const outcome = await sendArchive(intent);
      if (outcome) onarchive(intent, outcome);
    }, 'shelf-transition');
  }

  onMount(() => { void load(); });
</script>

<article class="space-y-4" aria-labelledby="context-detail-title">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <h2 id="context-detail-title" tabindex="-1" class="text-lg leading-tight font-semibold tracking-tight outline-none">Shelf archive state</h2>
    {#if observed}<Badge tone={observed.archived ? 'warning' : 'success'}>{observed.archived ? 'Archived' : 'Active'}</Badge>{/if}
  </div>
  <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
    <div class="min-w-0"><dt class="text-muted-foreground">Shelf ID</dt><dd class="font-mono text-xs break-all">{shelfId}</dd></div>
    <div class="min-w-0"><dt class="text-muted-foreground">Workspace ID</dt><dd class="font-mono text-xs break-all">{workspace?.id}</dd></div>
    {#if observed}<div><dt class="text-muted-foreground">Last read lifecycle revision</dt><dd class="tabular">{observed.revision}</dd></div>{/if}
  </dl>
  <div aria-live="polite">
    {#if error}
      <div class="space-y-2" role="alert">
        <p class="text-sm text-destructive">{error}</p>
        <Button variant="outline" size="sm" busy={app.working('shelf-state-retry')} onclick={() => app.run(load, 'shelf-state-retry')}><RotateCcw /> Retry</Button>
      </div>
    {:else if !observed}
      <Skeleton class="h-5 w-48" /><span class="sr-only">Loading shelf state</span>
    {:else}
      <p class="text-sm text-muted-foreground">This is an observation, not future authority.</p>
    {/if}
  </div>
  <p class="text-sm text-muted-foreground">Archiving hides all descendants together while keeping their IDs, ownership and history. Restoring keeps individually archived entries archived. A same-name collision is refused without renaming or overwriting. Archiving a shelf does not purge payloads or start entry cleanup.</p>

  {#if !scoped || workspace?.archived}
    <Alert>{workspace?.archived ? 'Shelf transitions require workspace reactivation. Read-only metadata remains available.' : 'Read-only. Use this workspace’s owner or admin key to manage shelves.'}</Alert>
  {/if}
  <div class="flex flex-wrap gap-2">
    {#if scoped && !workspace?.archived && observed}
      <Button variant={observed.archived ? 'default' : 'destructive'} busy={app.working('shelf-transition')} onclick={transition}>
        {#if observed.archived}<ArchiveRestore /> Restore shelf{:else}<Archive /> Archive shelf{/if}
      </Button>
    {/if}
    <Button variant="outline" onclick={onbrowse}>Browse archived shelves</Button>
  </div>
</article>
