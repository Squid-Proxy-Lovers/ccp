<script lang="ts">
  // Key numbers on one surface. The 1px grid gap over a border-coloured
  // background draws hairline dividers that hold in any column count.
  import MicroColumns from './MicroColumns.svelte';
  import {total, whole, bucketLabel, type ActivityDay, type Unit} from './types';
  import type {SessionStats} from '../../lib/types';

  let {stats, agents, days, unit}: {stats: SessionStats; agents: number; days: ActivityDay[]; unit: Unit} = $props();

  const writes = $derived(days.reduce((sum, day) => sum + total(day), 0));
  const created = $derived(days.reduce((sum, day) => sum + day.create, 0));
  const plural = (count: number, noun: string, nouns = `${noun}s`) => `${whole.format(count)} ${count === 1 ? noun : nouns}`;
  const cell = 'flex min-w-0 flex-col bg-card p-4 sm:p-5';
  const label = 'text-sm text-muted-foreground';
  const value = 'mt-3 text-[2rem] leading-none font-medium tracking-tight';
  const detail = 'mt-2.5 truncate text-xs text-muted-foreground';
</script>

<section aria-label="Key numbers" class="grid grid-cols-2 gap-px overflow-hidden rounded-xl border bg-border shadow-xs lg:grid-cols-5">
  <div class={cell}>
    <h2 class={label}>Entries</h2>
    <p class={value}>{whole.format(stats.entries)}</p>
    <p class={detail}>{created ? `+${whole.format(created)} in this window` : 'None created recently'}</p>
  </div>
  <div class={cell}>
    <h2 class={label}>Books</h2>
    <p class={value}>{whole.format(stats.books)}</p>
    <p class={detail}>On {plural(stats.shelves, 'shelf', 'shelves')}</p>
  </div>
  <div class={['col-span-2 lg:col-span-1', cell]}>
    <h2 class={label}>Agents reporting</h2>
    <p class={value}>{whole.format(agents)}</p>
    <p class={detail}>Unexpired status in this session</p>
  </div>
  <div class={['col-span-2 gap-4 sm:flex-row sm:items-end', cell]}>
    <div class="shrink-0">
      <h2 class={label}>Entry writes</h2>
      <p class={value}>{whole.format(writes)}</p>
      <p class={detail}>Since {days[0] ? bucketLabel(days[0].day, unit, true) : '—'}</p>
    </div>
    <div class="min-w-0 flex-1 sm:pl-4">
      <MicroColumns values={days.map(total)} />
      <div class="mt-1.5 flex justify-between text-[11px] text-muted-foreground">
        <span>{days[0] ? bucketLabel(days[0].day, unit) : ''}</span><span>Now</span>
      </div>
    </div>
  </div>
</section>
