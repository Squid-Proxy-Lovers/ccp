<script lang="ts">
  // Workspace dashboard: aggregate counts only (no entry content), plus the
  // newest reported work. The range filter scopes activity and writers.
  import {onMount, untrack} from 'svelte';
  import {RefreshCw, MessageSquareText} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardHeader from '../lib/ui/CardHeader.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import OverviewSkeleton from './overview/OverviewSkeleton.svelte';
  import Empty from '../lib/ui/Empty.svelte';
  import MetricBand from './overview/MetricBand.svelte';
  import ActivityCard from './overview/ActivityCard.svelte';
  import WritersCard from './overview/WritersCard.svelte';
  import AttentionCard from './overview/AttentionCard.svelte';
  import {total, whole, type Overview, type AttentionItem} from './overview/types';
  import {app} from '../lib/app.svelte';
  import {reportedWorkPage} from '../lib/model.js';
  import {relative, stamp} from '../lib/notify.svelte';
  import type {View} from '../lib/types';

  interface Report { status_id: string; client_name: string; agent: string | null; message: string; updated_at: number; stale: boolean; client_disabled: boolean }
  interface ReportPage { items: Report[]; next_cursor: string | null; stale_after_seconds: number }

  type Range = '7' | '30' | '90';
  let range = $state<Range>('30');
  let overview = $state.raw<Overview | null>(null);
  /** Writes in the window before this one, when twice the range fits the 90-day limit. */
  let previous = $state.raw<number | null>(null);
  let overviewError = $state('');
  let loading = $state(false);
  let reports = $state.raw<ReportPage | null>(null);
  let reportsError = $state('');

  const workspace = $derived(app.workspace);
  const days = $derived(Number(range));

  async function loadOverview() {
    if (!workspace) return;
    loading = true;
    try {
      const path = '/api/v1/admin/workspaces/overview';
      const [current, wide] = await Promise.all([
        app.request<Overview>(path, {workspace_id: workspace.id, days}),
        days * 2 <= 90 ? app.request<Overview>(path, {workspace_id: workspace.id, days: days * 2}) : null,
      ]);
      overview = current;
      // The wider read is oldest first, so its first half is the previous window.
      previous = wide ? wide.activity.slice(0, days).reduce((sum, day) => sum + total(day), 0) : null;
      overviewError = '';
    } catch (error) {
      if (!app.locked) overviewError = (error as Error).message;
    } finally { loading = false; }
  }

  async function loadReports() {
    if (!workspace) return;
    try {
      reports = reportedWorkPage(await app.request('/api/v1/work/status/list', {workspace_id: workspace.id, after: null, limit: 32}), workspace.id) as ReportPage;
      reportsError = '';
    } catch (error) {
      if (!app.locked) reportsError = (error as Error).message;
    }
  }

  // The view remounts on workspace change and on the header refresh; the
  // range is the only in-view input, and it keeps the previous render while loading.
  $effect(() => { void range; untrack(() => void loadOverview()); });
  onMount(() => void loadReports());

  const recent = $derived(reports ? [...reports.items].sort((a, b) => b.updated_at - a.updated_at).slice(0, 5) : []);
  const stale = $derived(reports?.items.filter(report => report.stale).length ?? 0);

  const attention = $derived.by(() => {
    const items: AttentionItem[] = [];
    if (workspace?.archived) items.push({tone: 'warning', title: 'Workspace is archived', detail: 'Content is read-only until an owner reactivates it.', view: 'settings'});
    if (overview) {
      const {access} = overview;
      if (access.credentials_expiring_within_7d) items.push({tone: 'warning', title: `${plural(access.credentials_expiring_within_7d, 'client credential')} expiring within 7 days`, detail: 'Clients lose access when a credential expires without renewal.', view: 'access'});
      if (!access.active_clients) items.push({tone: 'info', title: 'No clients have access yet', detail: 'Invite a client to start sharing context.', view: 'access'});
      if (access.pending_invitations) items.push({tone: 'info', title: `${plural(access.pending_invitations, 'invitation')} waiting to be redeemed`, detail: 'Revoke links you no longer need.', view: 'access'});
    }
    if (stale) items.push({tone: 'info', title: `${plural(stale, 'stale status message')}${reports?.next_cursor ? ' or more' : ''}`, detail: 'Stale reports keep their last text. They do not mean work stopped.', view: 'reported'});
    return items;
  });

  function plural(count: number, noun: string, nouns = `${noun}s`) { return `${whole.format(count)} ${count === 1 ? noun : nouns}`; }
  function open(view: View) { void app.run(() => app.navigate(view), `nav:${view}`); }
</script>

<div class="space-y-6">
  <PageHeader title="Overview" description={workspace ? `Activity and access in ${workspace.name}.` : undefined}>
    {#snippet meta()}
      {#if workspace?.archived}<Badge tone="warning">Archived</Badge>{/if}
      {#if overview}<span class="text-xs text-muted-foreground">Updated {stamp(overview.generated_at)}</span>{/if}
    {/snippet}
    {#snippet actions()}
      <Tabs label="Time range" bind:value={range}
        items={[{value: '7', label: '7 days'}, {value: '30', label: '30 days'}, {value: '90', label: '90 days'}]} />
    {/snippet}
  </PageHeader>

  {#if overviewError}
    <Alert tone="destructive" title="Couldn't load workspace activity" live>
      <p>{overviewError}</p>
      {#snippet action()}
        <Button variant="outline" size="sm" busy={loading} onclick={() => void loadOverview()}><RefreshCw /> Try again</Button>
      {/snippet}
    </Alert>
  {/if}

  {#if overview}
    <div class={['grid gap-6 transition-opacity', loading && 'opacity-60']} aria-busy={loading}>
      <MetricBand {overview} {days} {previous} />

      <div class="grid gap-6 lg:grid-cols-3">
        <div class="min-w-0 lg:col-span-2"><ActivityCard days={overview.activity} range={days} /></div>
        <WritersCard writers={overview.writers} range={days} />
      </div>
    </div>
  {:else if !overviewError}
    <Delayed><OverviewSkeleton /></Delayed>
  {/if}

  <div class="grid gap-6 lg:grid-cols-3">
    <AttentionCard items={attention} onopen={open} loading={(!overview && !overviewError) || (!reports && !reportsError)} />
    <Card class="min-w-0 lg:col-span-2">
      <CardHeader title="Reported work" description="Latest client status messages. They don't prove an agent is online or finished.">
        {#snippet action()}
          <Button variant="outline" size="sm" busy={app.working('nav:reported')} onclick={() => open('reported')}>View all</Button>
        {/snippet}
      </CardHeader>
      <CardContent>
        {#if reportsError}
          <p class="text-sm text-destructive">{reportsError}</p>
        {:else if !reports}
          <Delayed>
            <span class="sr-only" role="status">Loading reported work</span>
            <ul class="-my-1 divide-y" aria-hidden="true">
              {#each ['w-3/4', 'w-1/2', 'w-2/3'] as width, index (index)}
                <li class="flex items-start gap-4 py-3" aria-hidden="true">
                  <div class="flex-1 space-y-2"><Skeleton class="h-3.5 w-32" /><Skeleton class={['h-3 opacity-70', width]} /></div>
                  <div class="flex flex-col items-end gap-2"><Skeleton class="h-5 w-12" /><Skeleton class="h-3 w-16 opacity-70" /></div>
                </li>
              {/each}
            </ul>
          </Delayed>
        {:else if recent.length}
          <ul class="-my-1 divide-y">
            {#each recent as report (report.status_id)}
              <li class="flex items-start gap-4 py-3">
                <div class="min-w-0 flex-1 space-y-1">
                  <p class="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
                    <span class="font-medium">{report.client_name}</span>
                    {#if report.agent}<span class="text-muted-foreground">· {report.agent}</span>{/if}
                    {#if report.client_disabled}<Badge tone="destructive">Client disabled</Badge>{/if}
                  </p>
                  <p class="line-clamp-2 text-sm break-words whitespace-pre-line text-muted-foreground">{report.message}</p>
                </div>
                <div class="flex shrink-0 flex-col items-end gap-1">
                  <Badge tone={report.stale ? 'neutral' : 'success'}>{report.stale ? 'Stale' : 'Fresh'}</Badge>
                  <time class="text-xs text-muted-foreground" datetime={new Date(report.updated_at * 1000).toISOString()} title={stamp(report.updated_at)}>{relative(report.updated_at)}</time>
                </div>
              </li>
            {/each}
          </ul>
        {:else}
          <Empty icon={MessageSquareText} title="No status messages" description="Clients can publish what they're working on. Silence doesn't mean work is done." />
        {/if}
      </CardContent>
    </Card>
  </div>
</div>
