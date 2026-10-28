<script lang="ts">
  // Sign-in. The key is cleared from the field as soon as it is submitted, and a
  // reusable link is only produced on request because it carries the key itself.
  import {Eye, EyeOff, Link2, ArrowRight, CircleAlert} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Logo from './Logo.svelte';
  import Appearance from './Appearance.svelte';
  import {app} from '../lib/app.svelte';
  import {wire} from '../lib/model.js';
  import {notice, copyText} from '../lib/notify.svelte';

  interface SoftwareInfo { instance_id: string; package_version: string; native_wire: number; workspace_api: number }

  let key = $state('');
  let shown = $state(false);
  let error = $state('');
  let link = $state('');
  let info = $state.raw<SoftwareInfo | null>(null);
  let keyInput = $state<HTMLInputElement | null>(null);
  let linkInput = $state<HTMLInputElement | null>(null);
  const busy = $derived(app.working('login') || app.working('boot'));

  $effect(() => { keyInput?.focus(); });
  // Public, authority-free metadata so people can tell which server they are signing in to.
  $effect(() => {
    wire('/api/v1/software-info').then((value: SoftwareInfo) => {
      if (typeof value?.instance_id === 'string' && typeof value.package_version === 'string') info = value;
    }, () => {});
  });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const value = key.trim();
    if (!value) { error = 'Enter an admin key.'; keyInput?.focus(); return; }
    key = '';
    link = '';
    error = '';
    void app.run(async () => {
      try { await app.login(value); }
      catch (failure) {
        error = (failure as Error).message;
        keyInput?.focus();
      }
    }, 'login');
  }

  async function copyLink() {
    if (!key.trim()) { error = 'Enter your key first, then copy the link before signing in.'; keyInput?.focus(); return; }
    error = '';
    const value = `${location.origin}/admin#key=${encodeURIComponent(key.trim())}`;
    const generation = app.generation;
    if (await copyText(value)) { notice('Login link copied. Anyone with it has this key’s access.', 'success'); return; }
    if (generation === app.generation && key) {
      link = value;
      queueMicrotask(() => { linkInput?.focus(); linkInput?.select(); });
    }
  }
</script>

<div class="grid min-h-svh bg-background lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
  <!-- Poster: always ink, in either mode. -->
  <aside class="relative hidden flex-col [--primary:oklch(0.985_0_0)] [--primary-foreground:oklch(0.1_0_0)] justify-between overflow-hidden bg-[oklch(0.1_0_0)] p-10 text-[oklch(0.985_0_0)] lg:flex dark:border-r dark:border-border dark:bg-[oklch(0.07_0_0)]">
    <div class="flex items-center gap-2.5">
      <Logo class="size-8" />
      <span class="text-[15px] font-medium tracking-tight">Cephalopod Coordination Protocol</span>
    </div>

    <div class="max-w-xl">
      <p class="text-[clamp(2.5rem,4.2vw,4.25rem)] leading-[0.98] font-medium tracking-[-0.035em]">
        Shared context for agents, and the people who run them.
      </p>
      <p class="mt-6 max-w-md text-[15px] leading-relaxed text-[oklch(0.72_0_0)]">
        Govern access, curate what agents share, and review their history and reported work from one place.
      </p>
    </div>

    <dl class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 font-mono text-xs text-[oklch(0.62_0_0)]">
      {#if info}
        <dt>Instance</dt><dd class="truncate text-[oklch(0.86_0_0)]" title={info.instance_id}>{info.instance_id}</dd>
        <dt>Server</dt><dd class="text-[oklch(0.86_0_0)]">ccp-server {info.package_version} · workspace API {info.workspace_api} · wire {info.native_wire}</dd>
      {:else}
        <dt>Instance</dt><dd>—</dd>
      {/if}
    </dl>
  </aside>

  <main class="relative flex flex-col px-6 py-6 sm:px-10">
    <div class="flex items-center justify-between">
      <div class="flex items-center gap-2 lg:invisible">
        <Logo class="size-7" />
        <span class="text-sm font-medium tracking-tight">CCP</span>
      </div>
      <Appearance />
    </div>

    <div class="mx-auto flex w-full max-w-sm flex-1 flex-col justify-center py-12">
      <h1 id="page-title" tabindex="-1" class="text-[1.75rem] leading-tight font-medium tracking-tight outline-none">Sign in</h1>
      <p class="mt-2 text-sm text-muted-foreground">Use an instance or workspace admin key. The key decides which workspaces and actions you can use.</p>

      <form class="mt-8 grid gap-5" onsubmit={submit} novalidate>
        <div class="grid gap-2">
          <label for="key" class="text-sm font-medium">Admin key</label>
          <div class="relative">
            <Input id="key" type={shown ? 'text' : 'password'} autocomplete="off" spellcheck={false} bind:value={key} bind:ref={keyInput}
              oninput={() => { link = ''; error = ''; }} placeholder="ccp_admin_…" class="h-11 pr-11 font-mono"
              aria-invalid={error ? true : undefined} aria-describedby={error ? 'key-error' : undefined} />
            <button type="button" onclick={() => (shown = !shown)} aria-label={shown ? 'Hide key' : 'Show key'} aria-pressed={shown}
              class="absolute inset-y-0 right-0 flex w-11 cursor-pointer items-center justify-center rounded-r-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4">
              {#if shown}<EyeOff aria-hidden="true" />{:else}<Eye aria-hidden="true" />{/if}
            </button>
          </div>
          {#if error}
            <p id="key-error" role="alert" class="flex items-start gap-1.5 text-sm text-destructive">
              <CircleAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />{error}
            </p>
          {/if}
        </div>
        <Button type="submit" size="lg" class="h-11 w-full" {busy}>Sign in <ArrowRight /></Button>
      </form>

      <div class="mt-8 border-t pt-6">
        <button type="button" onclick={copyLink}
          class="inline-flex cursor-pointer items-center gap-2 rounded text-sm font-medium outline-none hover:underline hover:underline-offset-4 focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4">
          <Link2 aria-hidden="true" /> Copy a reusable login link
        </button>
        <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
          The link contains your key, so anyone with it has that key's access. Keep it private; revoking the key invalidates it.
        </p>
        {#if link}
          <div class="mt-4 grid gap-2">
            <label for="login-link" class="text-sm font-medium">Reusable login link</label>
            <Input id="login-link" readonly value={link} bind:ref={linkInput} class="font-mono text-xs" />
          </div>
        {/if}
      </div>
    </div>

    <p class="text-center text-xs text-muted-foreground lg:hidden">
      {#if info}<span class="font-mono">ccp-server {info.package_version}</span>{/if}
    </p>
  </main>
</div>
