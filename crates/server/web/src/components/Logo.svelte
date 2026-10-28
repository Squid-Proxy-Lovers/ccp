<script lang="ts">
  // `alive` adds a blink and a tentacle sway (CSS in app.css, off under reduced motion).
  let {class: className = '', alive = false}: {class?: string; alive?: boolean} = $props();
</script>
<svg viewBox="0 0 32 32" class={[className, alive && 'logo-alive']} aria-hidden="true" fill="none">
  <rect width="32" height="32" rx="8" class="fill-primary" />
  <path d="M10 13.5a6 6 0 0 1 12 0v3.2c0 .9-.7 1.6-1.6 1.6H11.6c-.9 0-1.6-.7-1.6-1.6z" class="fill-primary-foreground" />
  <path d="M12 18.5v4.5" class="logo-tentacle stroke-primary-foreground" stroke-width="1.8" stroke-linecap="round" />
  <path d="M16 18.5v5.5" class="logo-tentacle logo-tentacle-2 stroke-primary-foreground" stroke-width="1.8" stroke-linecap="round" />
  <path d="M20 18.5v4.5" class="logo-tentacle logo-tentacle-3 stroke-primary-foreground" stroke-width="1.8" stroke-linecap="round" />
  <circle cx="13.6" cy="14.4" r="1.2" class="logo-eye fill-primary" /><circle cx="18.4" cy="14.4" r="1.2" class="logo-eye fill-primary" />
</svg>
