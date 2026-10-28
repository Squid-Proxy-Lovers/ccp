<script lang="ts">
  // Workspace settings. Sections appear only for the scope and role that can
  // use them: transfer for the workspace owner, the danger zone for the owner
  // or an instance key. Each write keeps the original panel's safety rules.
  import PageHeader from '../components/PageHeader.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import {app} from '../lib/app.svelte';
  import {canChangeLifecycle} from '../lib/model.js';
  import DangerZone from './settings/DangerZone.svelte';
  import Details from './settings/Details.svelte';
  import Lifecycle from './settings/Lifecycle.svelte';
  import PendingRequest from './settings/PendingRequest.svelte';
  import Retention from './settings/Retention.svelte';
  import Transfer from './settings/Transfer.svelte';
  import {SettingsState} from './settings/state.svelte';

  const ctl = new SettingsState();
  void ctl.init();
</script>

{#if app.workspace && app.session}
  {@const workspace = app.workspace}
  {@const session = app.session}
  <div class="space-y-6">
    <PageHeader title="Settings" description="Workspace details, lifecycle, retention and ownership.">
      {#snippet meta()}
        <Badge tone={workspace.archived ? 'warning' : 'success'}>{workspace.archived ? 'Archived' : 'Active'}</Badge>
        {#if session.identity.scope.kind === 'instance'}
          <Badge tone="outline">Instance key · read-only content</Badge>
        {:else}
          <Badge tone="outline">Workspace {session.identity.scope.role}</Badge>
        {/if}
      {/snippet}
    </PageHeader>

    {#if ctl.uncertain}<PendingRequest {ctl} uncertain={ctl.uncertain} />{/if}
    {#if ctl.frozen && !ctl.uncertain && !ctl.preview}
      <p class="text-sm text-muted-foreground" role="status">Controls are paused until the current state is read again. Use Refresh.</p>
    {/if}

    <div class="grid max-w-4xl gap-6">
      <Details {workspace} {session} />
      <Lifecycle {ctl} {workspace} {session} />
      <Retention {ctl} {workspace} />
      {#if ctl.ownerTools}<Transfer {ctl} {workspace} {session} />{/if}
      {#if canChangeLifecycle(session, workspace)}<DangerZone {ctl} {workspace} {session} />{/if}
    </div>
  </div>
{/if}
