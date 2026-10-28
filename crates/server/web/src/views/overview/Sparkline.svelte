<script lang="ts">
  // Trend only; the exact values live in the activity card.
  let {values}: {values: number[]} = $props();
  const W = 120, H = 32, PAD = 4;
  const peak = $derived(Math.max(1, ...values));
  const points = $derived(values.map((value, index) => [
    PAD + (values.length > 1 ? (index / (values.length - 1)) * (W - PAD * 2) : (W - PAD * 2) / 2),
    H - PAD - (value / peak) * (H - PAD * 2),
  ] as const));
  const last = $derived(points.at(-1));
</script>

<svg viewBox={`0 0 ${W} ${H}`} width={W} height={H} aria-hidden="true" class="overflow-visible">
  <polyline points={points.map(point => point.join(',')).join(' ')} fill="none" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" class="stroke-muted-foreground/40" vector-effect="non-scaling-stroke" />
  {#if last}<circle cx={last[0]} cy={last[1]} r="4" stroke-width="2" class="fill-foreground stroke-card" />{/if}
</svg>
