<script lang="ts">
  import Card from '../../lib/ui/Card.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import Tabs from '../../lib/ui/Tabs.svelte';
  import {table} from '../../lib/ui/table';
  import ActivityChart from './ActivityChart.svelte';
  import {SERIES, total, whole, dayLabel, type ActivityDay} from './types';

  let {days, range}: {days: ActivityDay[]; range: number} = $props();
  let mode = $state<'chart' | 'table'>('chart');
  const sums = $derived(SERIES.map(series => ({...series, sum: days.reduce((acc, day) => acc + series.value(day), 0)})));
  const grand = $derived(days.reduce((acc, day) => acc + total(day), 0));
  const rows = $derived([...days].reverse());
</script>

<Card class="min-w-0">
  <CardHeader title="Content activity" description={`Entry writes per UTC day, last ${range} days`}>
    {#snippet action()}
      <Tabs label="Activity display" bind:value={mode} items={[{value: 'chart', label: 'Chart'}, {value: 'table', label: 'Table'}]} />
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    <ul class="flex flex-wrap gap-x-5 gap-y-2 text-sm" aria-label="Totals by action">
      {#each sums as series (series.key)}
        <li class="flex items-center gap-2">
          <span class={['size-2.5 shrink-0 rounded-[3px]', series.swatch]} aria-hidden="true"></span>
          <span class="text-muted-foreground">{series.label}</span>
          <span class="tabular font-medium">{whole.format(series.sum)}</span>
        </li>
      {/each}
    </ul>
    {#if mode === 'chart'}
      <div class="relative">
        <ActivityChart {days} />
        {#if grand === 0}
          <p class="absolute inset-x-0 top-1/2 -translate-y-1/2 text-center text-sm text-muted-foreground">No entry writes in this period.</p>
        {/if}
      </div>
    {:else}
      <div class={[table.wrap, 'max-h-72 overflow-y-auto rounded-md border']}>
        <table class={table.table}>
          <caption class="sr-only">Entry writes per UTC day, newest first</caption>
          <thead class={[table.head, 'sticky top-0 bg-card']}>
            <tr>
              <th scope="col" class={table.headCell}>Day</th>
              {#each SERIES as series (series.key)}<th scope="col" class={[table.headCell, 'text-right']}>{series.label}</th>{/each}
              <th scope="col" class={[table.headCell, 'text-right']}>Total</th>
            </tr>
          </thead>
          <tbody class={table.body}>
            {#each rows as day (day.day)}
              <tr class={table.row}>
                <th scope="row" class={[table.cell, 'text-left font-normal whitespace-nowrap']}>{dayLabel(day.day, true)}</th>
                {#each SERIES as series (series.key)}<td class={[table.cell, 'tabular text-right']}>{whole.format(series.value(day))}</td>{/each}
                <td class={[table.cell, 'tabular text-right font-medium']}>{whole.format(total(day))}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </CardContent>
</Card>
