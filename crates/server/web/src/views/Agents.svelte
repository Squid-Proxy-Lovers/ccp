<script lang="ts">
  // Agent statuses for one session, or for every session (`all`). Statuses come
  // with the server overview, which refreshes while live updates are on.
  import {onMount} from 'svelte';
  import {Bot, Search} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import AgentList from './shared/AgentList.svelte';
  import {app} from '../lib/app.svelte';
  import {seconds} from '../lib/api';
  import {stamp} from '../lib/notify.svelte';

  let {all = false}: {all?: boolean} = $props();
  // svelte-ignore state_referenced_locally
  const name = all ? null : app.session;
  let query = $state('');

  onMount(() => { void app.loadOverview().catch(() => {}); });

  const items = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    return app.agents
      .filter(agent => !name || agent.session_name === name)
      .filter(agent => !needle || [agent.session_name, agent.team, agent.agent_name, agent.worker_id, agent.status].some(field => field.toLowerCase().includes(needle)))
      .sort((a, b) => seconds(b.updated_at) - seconds(a.updated_at));
  });
  const total = $derived(app.agents.filter(agent => !name || agent.session_name === name).length);
</script>

<div class="space-y-6">
  <PageHeader title={all ? 'All agents' : 'Agents'}
    description="Agents whose status has not expired. A status is whatever the agent last published; it doesn't prove the agent is still running.">
    {#snippet meta()}
      <span>{total} reporting</span>
      {#if app.loadedAt}<span>Updated {stamp(app.loadedAt)}</span>{/if}
    {/snippet}
  </PageHeader>

  {#if !app.live}
    <Alert title="Live updates paused"><p>Statuses refresh when you use the refresh button or turn live updates back on.</p></Alert>
  {/if}

  <Card>
    <CardContent class="space-y-4 pt-5">
      {#if total > 3}
        <div class="relative">
          <Search class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input type="search" bind:value={query} placeholder="Filter by agent, team, worker or status" aria-label="Filter agents" class="pl-9" />
        </div>
      {/if}
      {#if items.length}
        <AgentList {items} showSession={all} />
      {:else}
        <Empty icon={Bot} title={total ? 'No matching agents' : 'No agents reporting'}
          description={total ? 'Try a different filter.' : 'Agents publish status with set_status; it expires after its TTL.'} />
      {/if}
    </CardContent>
  </Card>
</div>
