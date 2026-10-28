<script lang="ts">
  // Signed in, but no workspace is open.
  import {Building2, ChevronRight, Globe} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import {app} from '../lib/app.svelte';
  import type {View, Workspace} from '../lib/types';

  function go(view: View) {
    void app.run(() => app.navigate(view), `nav:${view}`);
  }
  function open(workspace: Workspace) {
    void app.run(async () => { if (app.mayLeave()) await app.openWorkspace(workspace, 'overview'); }, `open:${workspace.id}`);
  }
</script>

<div class="space-y-6">
  <PageHeader title="No workspace open" />
  {#if app.workspaces.length}
    <Empty icon={Building2} title="Choose a workspace"
      description="Select a workspace to manage access or read shared context.">
      {#each app.workspaces.slice(0, 6) as workspace (workspace.id)}
        <Button variant="outline" size="sm" busy={app.working(`open:${workspace.id}`)} onclick={() => open(workspace)}>
          {workspace.name}{#if workspace.archived}<span class="text-muted-foreground"> · Archived</span>{/if} <ChevronRight />
        </Button>
      {/each}
      {#if app.instanceScope}
        <Button size="sm" busy={app.working('nav:workspaces')} onclick={() => go('workspaces')}>All workspaces</Button>
      {/if}
    </Empty>
  {:else}
    <Empty icon={Building2} title="No accessible workspace"
      description={app.instanceScope
        ? 'This instance has no workspaces yet. Create one from the workspace list.'
        : 'This key has no accessible workspace. It may have been deleted or your role removed. Ask the workspace owner or an instance admin.'}>
      {#if app.instanceScope}
        <Button size="sm" busy={app.working('nav:workspaces')} onclick={() => go('workspaces')}>Open workspace list</Button>
      {/if}
      <Button size="sm" variant="outline" busy={app.working('nav:global-guidance')} onclick={() => go('global-guidance')}><Globe /> Global guidance</Button>
    </Empty>
  {/if}
</div>
