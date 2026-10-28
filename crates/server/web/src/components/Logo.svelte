<script lang="ts">
  // The CCP mark: a closed almond eye with a horizontal pupil; its outer corner
  // runs on and coils like the tip of an arm. Static by default. It only blinks at launch
  // (`once`) or while something loads (`loading`); see app.css. With `watch`, the
  // pupil follows the mouse within the eye (CSSOM transform; off for reduced motion).
  // `closed` shuts the eye for good, for a session that has ended.
  let {class: className = '', motion = 'none', watch = false, closed = false}: {
    class?: string; motion?: 'none' | 'once' | 'loading'; watch?: boolean; closed?: boolean;
  } = $props();
  let svg = $state<SVGSVGElement | null>(null);
  let pupil = $state<SVGRectElement | null>(null);

  $effect(() => {
    if (!watch || !svg || !pupil || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const mark = svg, target = pupil;
    let frame = 0;
    const look = (event: PointerEvent) => {
      if (event.pointerType !== 'mouse') return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const box = mark.getBoundingClientRect();
        const dx = event.clientX - (box.left + box.width * 0.42);
        const dy = event.clientY - (box.top + box.height / 2);
        const distance = Math.hypot(dx, dy) || 1;
        // Ease toward the edge of the eye as the pointer moves away; stay inside the lids.
        const reach = Math.min(1, distance / 240);
        target.style.transform = `translate(${((dx / distance) * reach * 2.6).toFixed(2)}px, ${((dy / distance) * reach * 1.4).toFixed(2)}px)`;
      });
    };
    const rest = () => { target.style.transform = ''; };
    window.addEventListener('pointermove', look);
    document.documentElement.addEventListener('pointerleave', rest);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener('pointermove', look);
      document.documentElement.removeEventListener('pointerleave', rest);
    };
  });
</script>
<svg bind:this={svg} viewBox="0 0 32 32" class={[className, motion !== 'none' && `logo-${motion}`, closed && 'logo-closed']} aria-hidden="true" fill="none">
  <rect width="32" height="32" rx="7.5" class="fill-primary" />
  <g class="logo-eye">
    <path d="M22.6 16C20.4 11 17.8 8.4 14.6 8.4C11 8.4 7.6 10.9 4.4 16C7.6 21.1 11 23.6 14.6 23.6C17.8 23.6 20.4 21 22.6 16C24 12.8 28 12.5 28.2 15.9C28.3 18.1 26.4 18.9 25.2 17.8"
      class="stroke-primary-foreground" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round" />
    <rect bind:this={pupil} x="9.2" y="14.3" width="8.6" height="3.4" rx="1.7" class="logo-pupil fill-primary-foreground" />
  </g>
  {#if closed}
    <path d="M4.6 14.2C7.8 17.6 11 19 14.4 19C17.6 19 20.2 17.6 22.6 14.6C24 11.8 28 11.5 28.2 14.9C28.3 17.1 26.4 17.9 25.2 16.8M8.2 17.6L6.8 19.6M14.4 19V21.4M20.4 17.4L21.8 19.4"
      class="logo-lid stroke-primary-foreground" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round" />
  {/if}
</svg>
