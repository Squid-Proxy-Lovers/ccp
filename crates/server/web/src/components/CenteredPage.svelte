<script lang="ts">
  // Frame for the standalone screens (setup, join): a centred column on the faint
  // sign-in dot grid, with the appearance menu in the corner.
  import type {Snippet} from 'svelte';
  import Appearance from './Appearance.svelte';
  let {children, busy = false}: {children: Snippet; busy?: boolean} = $props();
</script>
<div class="relative isolate flex min-h-svh flex-col items-center justify-center gap-8 overflow-hidden bg-background px-6 py-16" aria-busy={busy}>
  <div class="login-dots pointer-events-none -z-10" aria-hidden="true"></div>
  <div class="absolute top-4 right-4 z-10"><Appearance /></div>
  {@render children()}
</div>
