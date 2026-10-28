<script lang="ts">
  import {Link2, ShieldCheck} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Logo from './Logo.svelte';
  import Appearance from './Appearance.svelte';
  import {app} from '../lib/app.svelte';
  import {notice, copyText} from '../lib/notify.svelte';

  let key = $state('');
  let link = $state('');
  let keyInput = $state<HTMLInputElement | null>(null);
  let linkInput = $state<HTMLInputElement | null>(null);
  $effect(() => { keyInput?.focus(); });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const value = key;
    key = '';
    link = '';
    void app.run(() => app.login(value), 'login');
  }
  async function copyLink() {
    if (!key) { notice('Enter your key first, then copy the link before signing in.', 'warning'); return; }
    const value = `${location.origin}/admin#key=${encodeURIComponent(key)}`;
    const generation = app.generation;
    if (!await copyText(value) && generation === app.generation && key) {
      link = value;
      queueMicrotask(() => { linkInput?.focus(); linkInput?.select(); });
    }
  }
</script>

<div class="relative flex min-h-svh flex-col items-center justify-center gap-6 bg-muted/40 p-6">
  <div class="absolute top-4 right-4"><Appearance /></div>
  <div class="flex items-center gap-2.5">
    <Logo class="size-9" />
    <span class="text-lg font-semibold tracking-tight">CCP</span>
  </div>
  <div class="w-full max-w-sm rounded-xl border bg-card p-6 shadow-sm">
    <div class="mb-6 space-y-1.5">
      <h1 id="page-title" tabindex="-1" class="text-xl font-semibold tracking-tight outline-none">Sign in</h1>
      <p class="text-sm text-muted-foreground">Use an instance or workspace admin key. The key decides which workspaces and actions you can use.</p>
    </div>
    <form class="grid gap-4" onsubmit={submit}>
      <Field id="key" label="Admin key">
        <Input id="key" type="password" required autocomplete="off" spellcheck={false} bind:value={key} bind:ref={keyInput}
          oninput={() => (link = '')} placeholder="ccp_admin_…" class="font-mono" />
      </Field>
      <Button type="submit" class="w-full" busy={app.working('login') || app.working('boot')}>Sign in</Button>
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
  <p class="flex max-w-sm items-start gap-2 text-xs leading-relaxed text-muted-foreground">
    <ShieldCheck class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
    A reusable link contains your key, so anyone with it has that key's access. Keep it private and revoke the key to invalidate it.
  </p>
</div>
