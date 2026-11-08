<script lang="ts">
  // Activity feed for one session, or for every session (`all`). Polls while
  // live updates are on; new rows appear at the top.
  import {onMount} from 'svelte';
  import {RefreshCw, Search, MessageSquareText} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Select from '../lib/ui/Select.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import SkeletonTable from '../lib/ui/skeleton/SkeletonTable.svelte';
  import ActivityList from './shared/ActivityList.svelte';
  import {app} from '../lib/app.svelte';
  import {enc} from '../lib/api';
  import {whole} from './overview/types';
  import type {Activity} from '../lib/types';

  let {all = false}: {all?: boolean} = $props();

  // svelte-ignore state_referenced_locally
  const name = all ? null : app.session;
  let items = $state.raw<Activity[] | null>(null);
  let failed = $state('');
  let loading = $state(false);
  let query = $state('');
  let kind = $state<'all' | 'entry_created' | 'entry_appended'>('all');
  let limit = $state('200');
  let shown = $state(50);

  async function load() {
    const generation = app.generation;
    loading = true;
    try {
      const path = name ? `/v1/admin/activity?session=${enc(name)}&limit=${limit}` : `/v1/admin/activity?limit=${limit}`;
      const next = await app.request<Activity[]>(path);
      if (generation !== app.generation) return;
      items = next;
      failed = '';
    } catch (error) {
      if (!app.locked && generation === app.generation) failed = (error as Error).message;
    } finally { loading = false; }
  }
  onMount(() => { void load(); });
  $effect(() => {
    if (!app.live) return;
    const timer = setInterval(() => { if (document.visibilityState === 'visible' && !loading) void load(); }, 5000);
    return () => clearInterval(timer);
  });

  const filtered = $derived.by(() => {
    if (!items) return [];
    const needle = query.trim().toLowerCase();
    return items.filter(item => (kind === 'all' || item.kind === kind) && (!needle || [
      item.session_name, item.shelf_name, item.book_name, item.entry_name, item.actor, item.agent_name ?? '', item.host_name ?? '', item.reason ?? '', item.content,
    ].some(field => field.toLowerCase().includes(needle))));
  });
  const counts = $derived({
    all: items?.length ?? 0,
    created: items?.filter(item => item.kind !== 'entry_appended').length ?? 0,
    appended: items?.filter(item => item.kind === 'entry_appended').length ?? 0,
  });
</script>

<div class="space-y-6">
  <PageHeader title={all ? 'Live activity' : 'Activity'}
    description={all ? 'Entry writes across every session on this server, newest first.' : `Entry writes in ${name}, newest first.`}>
    {#snippet meta()}
      {#if items}<span>{whole.format(items.length)} loaded</span>{/if}
      <span>{app.live ? 'Refreshing every 5 seconds' : 'Live updates paused'}</span>
    {/snippet}
    {#snippet actions()}
      <Select aria-label="How many writes to load" bind:value={limit} onchange={() => void load()}
        options={[{value: '100', label: 'Newest 100'}, {value: '200', label: 'Newest 200'}, {value: '500', label: 'Newest 500'}]} />
      <Button variant="outline" size="sm" busy={loading && items !== null} onclick={() => void load()}><RefreshCw /> Reload</Button>
    {/snippet}
  </PageHeader>

  {#if failed}
    <Alert tone="destructive" title="Couldn't load activity" live><p>{failed}</p></Alert>
  {/if}

  <Card>
    <CardContent class="space-y-4 pt-5">
      <div class="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div class="relative flex-1">
          <Search class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input type="search" bind:value={query} oninput={() => (shown = 50)} placeholder={all ? 'Filter by session, path, agent or content' : 'Filter by path, agent or content'} aria-label="Filter activity" class="pl-9" />
        </div>
        <Tabs label="Write type" bind:value={kind} onchange={() => (shown = 50)}
          items={[{value: 'all', label: 'All', count: counts.all}, {value: 'entry_created', label: 'Created', count: counts.created}, {value: 'entry_appended', label: 'Appended', count: counts.appended}]} />
      </div>

      {#if !items && !failed}
        <Delayed><SkeletonTable rows={6} label="Loading activity" flush /></Delayed>
      {:else if items && filtered.length}
        <ActivityList items={filtered.slice(0, shown)} showSession={all} />
        {#if filtered.length > shown}
          <div class="flex justify-center pt-2">
            <Button variant="outline" size="sm" onclick={() => (shown += 50)}>Show {whole.format(Math.min(50, filtered.length - shown))} more</Button>
          </div>
        {/if}
      {:else if items}
        <Empty icon={MessageSquareText} title={items.length ? 'No matching writes' : 'No writes yet'}
          description={items.length ? 'Try a different filter.' : 'Entries created or appended by agents appear here.'} />
      {/if}
    </CardContent>
  </Card>
</div>
