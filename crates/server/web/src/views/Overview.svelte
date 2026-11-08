<script lang="ts">
  // Session dashboard: totals, a chart of the newest writes, recent activity and
  // the agents currently reporting status.
  import {onMount} from 'svelte';
  import {RefreshCw, MessageSquareText, Bot} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardHeader from '../lib/ui/CardHeader.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import OverviewSkeleton from './overview/OverviewSkeleton.svelte';
  import MetricBand from './overview/MetricBand.svelte';
  import ActivityCard from './overview/ActivityCard.svelte';
  import ActivityList from './shared/ActivityList.svelte';
  import AgentList from './shared/AgentList.svelte';
  import {bucket} from './overview/types';
  import {app} from '../lib/app.svelte';
  import {enc} from '../lib/api';
  import {stamp} from '../lib/notify.svelte';
  import type {Activity, SessionStats, SessionView} from '../lib/types';

  // svelte-ignore state_referenced_locally
  const name = app.session as string;
  let stats = $state.raw<SessionStats | null>(app.current);
  let events = $state.raw<Activity[] | null>(null);
  let failed = $state('');
  let loading = $state(false);

  const agents = $derived(app.agents.filter(agent => agent.session_name === name));
  const chart = $derived(events ? bucket(events) : null);
  const recent = $derived(events?.slice(0, 6) ?? []);

  async function load() {
    const generation = app.generation;
    loading = true;
    try {
      const [nextStats, nextEvents] = await Promise.all([
        app.request<SessionStats>(`/v1/admin/sessions/${enc(name)}/stats`),
        app.request<Activity[]>(`/v1/admin/activity?session=${enc(name)}&limit=500`),
      ]);
      if (generation !== app.generation) return;
      stats = nextStats;
      events = nextEvents;
      failed = '';
    } catch (error) {
      if (!app.locked && generation === app.generation) failed = (error as Error).message;
    } finally { loading = false; }
  }
  onMount(() => { void load(); });
  $effect(() => {
    if (!app.live) return;
    const timer = setInterval(() => { if (document.visibilityState === 'visible' && !loading) void load(); }, 10000);
    return () => clearInterval(timer);
  });

  function open(view: SessionView) { void app.run(() => app.openSession(name, view), `nav:${view}`); }
</script>

<div class="space-y-6">
  <PageHeader title={name} description={stats?.session.purpose || stats?.session.description || undefined}>
    {#snippet meta()}
      {#if stats}
        <Badge tone={stats.is_active ? 'success' : 'neutral'}>{stats.is_active ? 'Active' : 'Inactive'}</Badge>
        <span>{stats.session.visibility}</span>
        <span class="font-mono">#{stats.session.session_id}</span>
        {#if stats.session.owner}<span>Owner {stats.session.owner}</span>{/if}
        {#each stats.session.labels as label (label)}<span>{label}</span>{/each}
      {/if}
      {#if app.loadedAt}<span>Updated {stamp(app.loadedAt)}</span>{/if}
    {/snippet}
  </PageHeader>

  {#if failed}
    <Alert tone="destructive" title="Couldn't load session activity" live>
      <p>{failed}</p>
      {#snippet action()}
        <Button variant="outline" size="sm" busy={loading} onclick={() => void load()}><RefreshCw /> Try again</Button>
      {/snippet}
    </Alert>
  {/if}

  {#if stats && chart && events}
    <div class="grid gap-6">
      <MetricBand {stats} agents={agents.length} days={chart.days} unit={chart.unit} />
      <ActivityCard days={chart.days} unit={chart.unit} sampled={events.length} />
    </div>
  {:else if !failed}
    <Delayed><OverviewSkeleton /></Delayed>
  {/if}

  {#if events}
    <div class="grid items-start gap-6 lg:grid-cols-5">
      <Card class="min-w-0 lg:col-span-3">
        <CardHeader title="Recent writes" description="Newest entry creations and appends in this session.">
          {#snippet action()}
            <Button variant="outline" size="sm" busy={app.working('nav:activity')} onclick={() => open('activity')}>View all</Button>
          {/snippet}
        </CardHeader>
        <CardContent>
          {#if recent.length}
            <ActivityList items={recent} clamp={2} />
          {:else}
            <Empty icon={MessageSquareText} title="No writes yet" description="Entries created or appended by agents appear here." />
          {/if}
        </CardContent>
      </Card>
      <Card class="min-w-0 lg:col-span-2">
        <CardHeader title="Agents" description="Agents with an unexpired status in this session.">
          {#snippet action()}
            <Button variant="outline" size="sm" busy={app.working('nav:agents')} onclick={() => open('agents')}>Details</Button>
          {/snippet}
        </CardHeader>
        <CardContent>
          {#if agents.length}
            <AgentList items={agents.slice(0, 6)} compact />
          {:else}
            <Empty icon={Bot} title="No agents reporting" description="Agents publish status with set_status. Silence doesn't mean work stopped." />
          {/if}
        </CardContent>
      </Card>
    </div>
  {/if}
</div>
