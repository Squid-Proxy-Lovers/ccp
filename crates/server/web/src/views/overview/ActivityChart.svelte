<script lang="ts">
  // Stacked daily columns. Geometry is plain SVG attributes (CSP-safe); the
  // tooltip is positioned through the CSSOM. Every value is also in the table view.
  import {css} from '../../lib/ui/css';
  import {SERIES, total, compact, whole, dayLabel, type ActivityDay} from './types';

  let {days}: {days: ActivityDay[]} = $props();

  const HEIGHT = 240, TOP = 8, BOTTOM = 26, LEFT = 36, RIGHT = 4, GAP = 2, RADIUS = 4;
  let width = $state(0);
  let active = $state<number | null>(null);

  const plotWidth = $derived(Math.max(0, width - LEFT - RIGHT));
  const plotHeight = HEIGHT - TOP - BOTTOM;
  const band = $derived(days.length ? plotWidth / days.length : 0);
  const barWidth = $derived(Math.min(24, Math.max(2, band - Math.max(2, band * 0.3))));

  const scale = $derived.by(() => {
    const peak = Math.max(0, ...days.map(total));
    const raw = Math.max(1, peak / 4);
    const magnitude = 10 ** Math.floor(Math.log10(raw));
    const step = Math.max(1, [1, 2, 5, 10].map(f => f * magnitude).find(s => s >= raw) ?? raw);
    const top = Math.max(step * 4, Math.ceil(peak / step) * step);
    const ticks: number[] = [];
    for (let value = 0; value <= top; value += step) ticks.push(value);
    return {top, ticks};
  });
  const y = (value: number) => TOP + plotHeight - (value / scale.top) * plotHeight;

  interface Segment { key: string; fill: string; x: number; y: number; w: number; h: number; path?: string }
  const columns = $derived(days.map((day, index) => {
    const x = LEFT + index * band + (band - barWidth) / 2;
    const segments: Segment[] = [];
    let sum = 0;
    const visible = SERIES.filter(series => series.value(day) > 0);
    visible.forEach((series, position) => {
      const value = series.value(day);
      const bottom = y(sum) - (position > 0 ? GAP : 0);
      sum += value;
      const top = y(sum);
      const h = Math.max(1, bottom - top);
      const segment: Segment = {key: series.key, fill: series.fill, x, y: bottom - h, w: barWidth, h};
      if (position === visible.length - 1) {
        // Rounded data end, square at the baseline side.
        const r = Math.min(RADIUS, barWidth / 2, h);
        const t = bottom - h;
        segment.path = `M${x},${bottom}V${t + r}Q${x},${t} ${x + r},${t}H${x + barWidth - r}Q${x + barWidth},${t} ${x + barWidth},${t + r}V${bottom}Z`;
      }
      segments.push(segment);
    });
    return {index, segments};
  }));

  const labelStep = $derived(days.length <= 7 ? 1 : days.length <= 31 ? 7 : 15);
  const labels = $derived(days.map((day, index) => ({index, day})).filter(({index}) => (days.length - 1 - index) % labelStep === 0));

  const activeDay = $derived(active === null ? null : days[active] ?? null);
  const tooltipLeft = $derived(active === null ? 0 : LEFT + active * band + band / 2);
  const flip = $derived(tooltipLeft > width / 2);
  const announcement = $derived(activeDay
    ? `${dayLabel(activeDay.day, true)}: ${whole.format(total(activeDay))} writes. ${SERIES.map(s => `${s.label} ${s.value(activeDay)}`).join(', ')}.`
    : '');

  function keydown(event: KeyboardEvent) {
    if (!days.length) return;
    const last = days.length - 1;
    const moves: Record<string, number> = {ArrowLeft: -1, ArrowRight: 1, Home: -Infinity, End: Infinity};
    const move = moves[event.key];
    if (move === undefined) return;
    event.preventDefault();
    active = Math.min(last, Math.max(0, (active ?? last) + move));
  }
</script>

<div class="relative" bind:clientWidth={width}>
  <!-- A focusable chart region: arrow keys move the readout, announced via the live region below. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div role="group" tabindex="0" aria-label="Daily writes chart. Use the left and right arrow keys to read each day."
    class="rounded-md outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
    onkeydown={keydown} onfocus={() => { active ??= days.length - 1; }} onblur={() => { active = null; }}
    onpointerleave={() => { active = null; }}>
    {#if width > 0}
      <svg {width} height={HEIGHT} aria-hidden="true" class="block overflow-visible">
        {#each scale.ticks as tick (tick)}
          <line x1={LEFT} x2={width - RIGHT} y1={y(tick)} y2={y(tick)} class="stroke-border" stroke-width="1" shape-rendering="crispEdges" />
          <text x={LEFT - 8} y={y(tick)} dy="0.32em" text-anchor="end" class="tabular fill-muted-foreground text-[11px]">{compact.format(tick)}</text>
        {/each}
        {#if active !== null}
          <rect x={LEFT + active * band} y={TOP} width={band} height={plotHeight} rx="3" class="fill-foreground/[0.05]" />
        {/if}
        {#each columns as column (column.index)}
          {#each column.segments as segment (segment.key)}
            {#if segment.path}
              <path d={segment.path} class={segment.fill} />
            {:else}
              <rect x={segment.x} y={segment.y} width={segment.w} height={segment.h} class={segment.fill} />
            {/if}
          {/each}
        {/each}
        {#each labels as label (label.index)}
          <text x={LEFT + label.index * band + band / 2} y={HEIGHT - 6} text-anchor="middle" class="fill-muted-foreground text-[11px]">{dayLabel(label.day.day)}</text>
        {/each}
        {#each days as _, index (index)}
          <rect role="presentation" x={LEFT + index * band} y={TOP} width={band} height={plotHeight} fill="transparent"
            onpointerenter={() => { active = index; }} />
        {/each}
      </svg>
    {:else}
      <div class="h-60"></div>
    {/if}
  </div>

  {#if activeDay}
    <div use:css={{left: `${tooltipLeft}px`, transform: flip ? 'translateX(calc(-100% - 12px))' : 'translateX(12px)'}}
      class="pointer-events-none absolute top-2 z-10 min-w-44 rounded-lg border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md">
      <p class="mb-1.5 font-medium">{dayLabel(activeDay.day, true)}</p>
      <ul class="grid gap-1">
        {#each SERIES as series (series.key)}
          <li class="flex items-center gap-2">
            <svg width="12" height="2" aria-hidden="true" class="shrink-0"><line x1="0" x2="12" y1="1" y2="1" stroke-width="2" stroke-linecap="round" class={series.stroke} /></svg>
            <span class="tabular font-semibold text-foreground">{whole.format(series.value(activeDay))}</span>
            <span class="text-muted-foreground">{series.label}</span>
          </li>
        {/each}
      </ul>
      <p class="mt-1.5 flex justify-between gap-4 border-t pt-1.5 text-muted-foreground">Total <span class="tabular font-semibold text-foreground">{whole.format(total(activeDay))}</span></p>
    </div>
  {/if}
  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>
