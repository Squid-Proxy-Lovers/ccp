<script lang="ts">
  // The CCP mark: an almond eye with a horizontal pupil whose lower lid runs on
  // and curls like the tip of an arm. Static by default. It only blinks at launch
  // (`once`) or while something loads (`loading`); see app.css.
  let {class: className = '', motion = 'none'}: {class?: string; motion?: 'none' | 'once' | 'loading'} = $props();
</script>
<svg viewBox="0 0 32 32" class={[className, motion !== 'none' && `logo-${motion}`]} aria-hidden="true" fill="none">
  <rect width="32" height="32" rx="7.5" class="fill-primary" />
  <g class="logo-eye">
    <path d="M26.5 15.2C23.2 10 19.6 7.8 16 7.8C12 7.8 8 10.2 4.5 16C8 21.8 12 24.2 16 24.2C19.4 24.2 22.4 22.4 24.6 20.4C26.6 18.6 28.6 19.6 28.2 21.6C27.9 23 26.4 23.4 25.6 22.6"
      class="stroke-primary-foreground" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" />
    <rect x="11.2" y="14.4" width="9.6" height="3.2" rx="1.6" class="fill-primary-foreground" />
  </g>
</svg>
