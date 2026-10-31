<script lang="ts">
  // Key numbers on one surface. The 1px grid gap over a border-coloured
  // background draws hairline dividers that hold in any column count.
  import {ArrowUpRight, ArrowDownRight, Minus} from '@lucide/svelte';
  import MicroColumns from './MicroColumns.svelte';
  import {total, whole, type Overview} from './types';

  let {overview, days, previous}: {overview: Overview; days: number; previous: number | null} = $props();

  const writes = $derived(overview.activity.reduce((sum, day) => sum + total(day), 0));
  const created = $derived(overview.activity.reduce((sum, day) => sum + day.create, 0));
  const change = $derived(previous === null ? null : writes - previous);
  const plural = (count: number, noun: string, nouns = `${noun}s`) => `${whole.format(count)} ${count === 1 ? noun : nouns}`;
  const accessDetail = $derived.by(() => {
    const {pending_invitations: pending, credentials_expiring_within_7d: expiring} = overview.access;
    const parts = [pending ? `${plural(pending, 'invitation')} pending` : 'No pending invitations'];
    if (expiring) parts.push(`${whole.format(expiring)} expiring soon`);
    return parts.join(' · ');
  });
  const cell = 'flex min-w-0 flex-col bg-card p-4 sm:p-5';
  const label = 'text-sm text-muted-foreground';
  const value = 'mt-3 text-[2rem] leading-none font-medium tracking-tight';
  const detail = 'mt-2.5 truncate text-xs text-muted-foreground';
</script>

<section aria-label="Key numbers" class="grid grid-cols-2 gap-px overflow-hidden rounded-xl border bg-border shadow-xs lg:grid-cols-5">
  <div class={cell}>
    <h2 class={label}>Entries</h2>
    <p class={value}>{whole.format(overview.totals.entries)}</p>
    <p class={detail}>{created ? `+${whole.format(created)} created` : 'None created'} · {whole.format(overview.totals.archived_entries)} archived</p>
  </div>
  <div class={cell}>
    <h2 class={label}>Books</h2>
    <p class={value}>{whole.format(overview.totals.books)}</p>
    <p class={detail}>On {plural(overview.totals.shelves, 'active shelf', 'active shelves')}</p>
  </div>
  <div class={['col-span-2 lg:col-span-1', cell]}>
    <h2 class={label}>Active clients</h2>
    <p class={value}>{whole.format(overview.access.active_clients)}</p>
    <p class={detail} title={accessDetail}>{accessDetail}</p>
  </div>
  <div class={['col-span-2 gap-4 sm:flex-row sm:items-end', cell]}>
    <div class="shrink-0">
      <h2 class={label}>Entry writes</h2>
      <p class={value}>{whole.format(writes)}</p>
      <p class={['flex items-center gap-1', detail]}>
        {#if change === null}
          Last {days} days
        {:else}
          {#if change > 0}<ArrowUpRight class="size-3.5 text-foreground" aria-hidden="true" />{:else if change < 0}<ArrowDownRight class="size-3.5 text-foreground" aria-hidden="true" />{:else}<Minus class="size-3.5" aria-hidden="true" />{/if}
          <span><span class="font-medium text-foreground">{change > 0 ? '+' : ''}{whole.format(change)}</span> vs previous {days} days</span>
        {/if}
      </p>
    </div>
    <div class="min-w-0 flex-1 sm:pl-4">
      <MicroColumns values={overview.activity.map(total)} />
      <div class="mt-1.5 flex justify-between text-[11px] text-muted-foreground"><span>{days} days ago</span><span>Today</span></div>
    </div>
  </div>
</section>
