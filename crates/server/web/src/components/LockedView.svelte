<script lang="ts">
  // Shown when the server stops accepting the admin key mid-session.
  import {ArrowRight} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Logo from './Logo.svelte';
  import {app} from '../lib/app.svelte';
</script>
<div class="relative isolate flex min-h-svh flex-col items-center justify-center gap-10 overflow-hidden bg-background p-6">
  <div class="login-dots pointer-events-none -z-10" aria-hidden="true"></div>
  <div class="rise flex max-w-sm flex-col items-center text-center">
    <div class="relative mb-7">
      <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
      <Logo class="size-16 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" closed />
    </div>
    <h1 id="page-title" tabindex="-1" class="text-2xl font-semibold tracking-tight outline-none">Session ended</h1>
    <p role="alert" class="mt-2 text-sm text-pretty text-muted-foreground">{app.lockMessage}</p>
    <Button class="group mt-7 min-w-44" onclick={() => app.signOut()}>
      Sign in again <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" />
    </Button>
  </div>
</div>
