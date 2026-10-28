<script lang="ts">
  // Archive or reactivate. The request has no operation ID and is never resent.
  import {Archive, ArchiveRestore} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import {app} from '../../lib/app.svelte';
  import {canChangeLifecycle} from '../../lib/model.js';
  import type {Session, Workspace} from '../../lib/types';
  import type {SettingsState} from './state.svelte';

  let {ctl, workspace, session}: {ctl: SettingsState; workspace: Workspace; session: Session} = $props();
  const allowed = $derived(canChangeLifecycle(session, workspace));
</script>

<Card>
  <CardHeader title="Workspace lifecycle" description={workspace.archived
    ? 'Archived: context remains readable. Content writes and new admission stop; timed entry cleanup is paused.'
    : 'Active: authorized content writes and admission are available. Explicitly enabled timed cleanup runs.'}>
    {#snippet action()}
      <Badge tone={workspace.archived ? 'warning' : 'success'}>{workspace.archived ? 'Archived' : 'Active'}</Badge>
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    {#if ctl.lifecycleAck}
      <Alert tone="success" title="Lifecycle action acknowledged" live>{ctl.lifecycleAck}</Alert>
    {/if}
    {#if ctl.lifecycleNote}<p class="text-sm" role="status">{ctl.lifecycleNote}</p>{/if}
    {#if allowed}
      <Button variant={workspace.archived ? 'default' : 'outline'} disabled={ctl.frozen} busy={app.working('lifecycle')}
        onclick={() => app.run(() => ctl.toggleLifecycle(), 'lifecycle')}>
        {#if workspace.archived}<ArchiveRestore /> Reactivate workspace{:else}<Archive /> Archive workspace{/if}
      </Button>
    {:else}
      <p class="text-sm text-muted-foreground">Only this workspace’s owner or an authorized instance key can archive or reactivate it.</p>
    {/if}
  </CardContent>
</Card>
