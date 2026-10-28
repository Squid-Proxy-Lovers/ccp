<script lang="ts">
  // Read-only workspace identity. The server reports no creation time today;
  // the row shows one only if the inspection carries it.
  import {Copy} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText} from '../../lib/notify.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import type {Session, Workspace} from '../../lib/types';

  let {workspace, session}: {workspace: Workspace; session: Session} = $props();

  const owner = $derived(app.inspection?.owner_id ?? null);
  const ownSession = $derived(owner !== null && owner === session.identity.principal_id);
</script>

<Card>
  <CardHeader title="Workspace details" description="Identifiers for support requests, CLI commands and audit records." />
  <CardContent>
    <dl class="grid gap-x-6 gap-y-4 text-sm sm:grid-cols-[10rem_1fr]">
      <dt class="text-muted-foreground">Name</dt>
      <dd class="font-medium break-words">{workspace.name}</dd>
      <dt class="self-center text-muted-foreground">Workspace ID</dt>
      <dd class="flex min-w-0 items-center gap-2">
        <code class="truncate font-mono text-[13px]">{workspace.id}</code>
        <Button variant="ghost" size="icon-sm" aria-label="Copy workspace ID" onclick={() => copyText(workspace.id)}><Copy /></Button>
      </dd>
      <dt class="text-muted-foreground">Owner</dt>
      <dd class="min-w-0">
        {#if !app.inspection}
          <Skeleton class="mt-0.5 h-4 w-72 max-w-full" /><span class="sr-only">Loading owner</span>
        {:else if owner}
          <code class="font-mono text-[13px] break-all">{owner}</code>
          {#if ownSession}<span class="text-muted-foreground"> (you)</span>{/if}
        {:else}
          Not appointed
        {/if}
      </dd>
    </dl>
  </CardContent>
</Card>
