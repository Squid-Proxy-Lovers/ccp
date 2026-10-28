<script lang="ts">
  // Sign-in. The key is cleared from the field as soon as it is submitted, and a
  // reusable link is only produced on request because it carries the key itself.
  import {Eye, EyeOff, Link2, ShieldCheck, CircleAlert, ArrowRight} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Logo from './Logo.svelte';
  import Appearance from './Appearance.svelte';
  import {app} from '../lib/app.svelte';
  import {wire} from '../lib/model.js';
  import {copyText} from '../lib/notify.svelte';

  let key = $state('');
  let shown = $state(false);
  let error = $state('');
  let link = $state('');
  let version = $state('');
  let keyInput = $state<HTMLInputElement | null>(null);
  let linkInput = $state<HTMLInputElement | null>(null);
  let card = $state<HTMLDivElement | null>(null);
  let backdrop = $state<HTMLDivElement | null>(null);
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');

  // Spotlight: the pointer position goes to CSS variables through the CSSOM
  // (allowed by the CSP), at most once per frame. Off for touch and reduced motion.
  $effect(() => {
    if (!backdrop || reduced.matches) return;
    const node = backdrop;
    let frame = 0;
    const move = (event: PointerEvent) => {
      if (event.pointerType !== 'mouse') return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        node.style.setProperty('--x', `${event.clientX}px`);
        node.style.setProperty('--y', `${event.clientY}px`);
        node.style.setProperty('--spot', '1');
      });
    };
    const leave = () => node.style.setProperty('--spot', '0');
    window.addEventListener('pointermove', move);
    document.documentElement.addEventListener('pointerleave', leave);
    return () => { cancelAnimationFrame(frame); window.removeEventListener('pointermove', move); document.documentElement.removeEventListener('pointerleave', leave); };
  });

  function shake() {
    if (reduced.matches) return;
    card?.animate([{transform: 'translateX(0)'}, {transform: 'translateX(-7px)'}, {transform: 'translateX(6px)'}, {transform: 'translateX(-4px)'}, {transform: 'translateX(2px)'}, {transform: 'translateX(0)'}],
      {duration: 420, easing: 'cubic-bezier(0.36, 0.07, 0.19, 0.97)'});
  }

  $effect(() => { keyInput?.focus(); });
  // Public, authority-free metadata: which server this is.
  $effect(() => {
    wire('/api/v1/software-info').then((value: {package_version?: unknown}) => {
      if (typeof value?.package_version === 'string') version = value.package_version;
    }, () => {});
  });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const value = key.trim();
    if (!value) { error = 'Enter an admin key.'; shake(); keyInput?.focus(); return; }
    key = '';
    link = '';
    error = '';
    void app.run(async () => {
      try { await app.login(value); }
      catch (failure) {
        error = (failure as Error).message;
        shake();
        keyInput?.focus();
      }
    }, 'login');
  }

  async function copyLink() {
    if (!key.trim()) { error = 'Enter your key first, then copy the link before signing in.'; keyInput?.focus(); return; }
    error = '';
    const value = `${location.origin}/admin#key=${encodeURIComponent(key.trim())}`;
    const generation = app.generation;
    if (!await copyText(value) && generation === app.generation && key) {
      link = value;
      queueMicrotask(() => { linkInput?.focus(); linkInput?.select(); });
    }
  }
</script>

<div class="relative isolate flex min-h-svh flex-col items-center justify-center gap-6 overflow-hidden bg-background p-6">
  <div bind:this={backdrop} class="pointer-events-none absolute inset-0 -z-10" aria-hidden="true">
    <div class="login-dots"></div>
    <div class="login-spot"></div>
  </div>
  <div class="absolute top-4 right-4 z-10"><Appearance /></div>
  <div class="rise flex flex-col items-center gap-4">
    <div class="relative">
      <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
      <Logo class="size-14 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" motion="once" watch />
    </div>
    <div class="text-center">
      <p class="text-[2rem] leading-none font-semibold tracking-[-0.045em]">CCP</p>
      <p class="mt-2 font-mono text-xs text-muted-foreground">Cephalopod Coordination Protocol</p>
    </div>
  </div>
  <div bind:this={card} class="rise rise-2 w-full max-w-sm rounded-xl border bg-card/90 p-6 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]">
    <div class="mb-6 space-y-1.5">
      <h1 id="page-title" tabindex="-1" class="text-xl font-semibold tracking-tight outline-none">Sign in</h1>
      <p class="text-sm text-muted-foreground">Use an instance or workspace admin key. The key decides which workspaces and actions you can use.</p>
    </div>
    <form class="grid gap-4" onsubmit={submit} novalidate>
      <div class="grid gap-2">
        <label for="key" class="text-sm font-medium">Admin key</label>
        <div class="relative">
          <Input id="key" type={shown ? 'text' : 'password'} autocomplete="off" spellcheck={false} bind:value={key} bind:ref={keyInput}
            oninput={() => { link = ''; error = ''; }} placeholder="ccp_admin_…" class="pr-10 font-mono"
            aria-invalid={error ? true : undefined} aria-describedby={error ? 'key-error' : undefined} />
          <button type="button" onclick={() => (shown = !shown)} aria-label={shown ? 'Hide key' : 'Show key'} aria-pressed={shown}
            class="absolute inset-y-0 right-0 flex w-10 cursor-pointer items-center justify-center rounded-r-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4">
            {#if shown}<EyeOff aria-hidden="true" />{:else}<Eye aria-hidden="true" />{/if}
          </button>
        </div>
        {#if error}
          <p id="key-error" role="alert" class="flex items-start gap-1.5 text-sm text-destructive">
            <CircleAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{error}
          </p>
        {/if}
      </div>
      <Button type="submit" class="group w-full" busy={app.working('login') || app.working('boot')}>
        Sign in <ArrowRight class="transition-transform duration-200 group-hover:translate-x-0.5" />
      </Button>
      <Button variant="outline" class="w-full" onclick={copyLink}><Link2 /> Copy reusable login link</Button>
    </form>
    {#if link}
      <div class="mt-4">
        <Field id="login-link" label="Reusable login link (keep private)">
          <Input id="login-link" readonly value={link} bind:ref={linkInput} class="font-mono text-xs" />
        </Field>
      </div>
    {/if}
  </div>
  <p class="rise rise-3 max-w-sm text-center text-xs leading-relaxed text-muted-foreground">
    <ShieldCheck class="mr-1 inline size-3.5 -translate-y-px align-middle text-destructive" aria-hidden="true" />
    A reusable link contains your key, so anyone with it has that key's access. Keep it private and revoke the key to invalidate it.
  </p>
  {#if version}<p class="rise rise-4 absolute bottom-4 font-mono text-[11px] text-muted-foreground">ccp-server {version}</p>{/if}
</div>
