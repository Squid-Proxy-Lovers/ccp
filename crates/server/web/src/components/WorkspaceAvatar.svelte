<script lang="ts">
  // A deterministic avatar per workspace, derived from its ID only (renaming
  // keeps it). A 3x3 grid of Truchet tiles: each cell joins its edge midpoints
  // with two quarter-circle arcs in one of two orientations, so the arcs flow
  // into continuous curls. The ID also picks one of ten deep inks.
  let {id, class: className = ''}: {id: string; class?: string} = $props();

  // Deep, low-chroma inks; white strokes clear 4.5:1 on every one.
  const INKS = [
    'oklch(0.42 0.13 265)', 'oklch(0.43 0.09 160)', 'oklch(0.42 0.13 24)', 'oklch(0.48 0.1 75)', 'oklch(0.42 0.12 330)',
    'oklch(0.44 0.08 200)', 'oklch(0.4 0.03 250)', 'oklch(0.45 0.12 45)', 'oklch(0.44 0.08 120)', 'oklch(0.4 0.13 290)',
  ];
  const CELL = 10;

  // FNV-1a, then xorshift32 for a stable bit stream.
  function stream(text: string) {
    let hash = 0x811c9dc5;
    for (let index = 0; index < text.length; index++) hash = Math.imul(hash ^ text.charCodeAt(index), 0x01000193);
    let state = hash >>> 0 || 1;
    return () => { state ^= state << 13; state ^= state >>> 17; state ^= state << 5; return (state >>> 0) / 0x100000000; };
  }

  const art = $derived.by(() => {
    const next = stream(id);
    const ink = INKS[Math.floor(next() * INKS.length)] ?? INKS[0];
    const arcs: string[] = [];
    for (let row = 0; row < 3; row++) {
      for (let col = 0; col < 3; col++) {
        const x = col * CELL, y = row * CELL, h = CELL / 2;
        // Orientation A joins top+left and bottom+right; B joins top+right and bottom+left.
        if (next() < 0.5) {
          arcs.push(`M${x + h} ${y}A${h} ${h} 0 0 1 ${x} ${y + h}`, `M${x + CELL} ${y + h}A${h} ${h} 0 0 0 ${x + h} ${y + CELL}`);
        } else {
          arcs.push(`M${x + h} ${y}A${h} ${h} 0 0 0 ${x + CELL} ${y + h}`, `M${x} ${y + h}A${h} ${h} 0 0 1 ${x + h} ${y + CELL}`);
        }
      }
    }
    return {ink, path: arcs.join('')};
  });
</script>

<svg viewBox="0 0 30 30" class={['shrink-0 rounded-[24%]', className]} aria-hidden="true">
  <rect width="30" height="30" fill={art.ink} />
  <path d={art.path} fill="none" stroke="white" stroke-width="2.4" stroke-linecap="round" />
</svg>
