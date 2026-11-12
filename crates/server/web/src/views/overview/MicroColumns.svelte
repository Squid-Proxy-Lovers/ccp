<script lang="ts">
  // One column per day, scaled to the busiest day. Earlier days are muted and the
  // current day is ink. Empty days keep a hairline tick so the rhythm of the
  // period stays visible. Exact values live in the activity card and its table.
  let {values, height = 56}: {values: number[]; height?: number} = $props();
  let width = $state(0);
  const peak = $derived(Math.max(1, ...values));
  const band = $derived(values.length ? width / values.length : 0);
  const bar = $derived(Math.max(1.5, Math.min(10, band * 0.62)));
  const columns = $derived(values.map((value, index) => {
    const h = value > 0 ? Math.max(3, (value / peak) * height) : 1;
    return {index, x: index * band + (band - bar) / 2, y: height - h, h, empty: value === 0, today: index === values.length - 1};
  }));
</script>

<div class="w-full" bind:clientWidth={width} aria-hidden="true">
  {#if width > 0}
    <svg {width} {height} class="block">
      {#each columns as column (column.index)}
        <rect x={column.x} y={column.y} width={bar} height={column.h} rx={column.empty ? 0 : Math.min(2, bar / 2)}
          class={column.today && !column.empty ? 'fill-foreground' : column.empty ? 'fill-border' : 'fill-muted-foreground/45'} />
      {/each}
    </svg>
  {:else}
    <div class="h-14"></div>
  {/if}
</div>
