<script lang="ts">
  // An unconfirmed settings write. Nothing retries automatically; the exact
  // original request stays visible until it is resolved or explicitly forgotten.
  // Only retention changes (operation ID + base revision) offer an exact re-send.
  import {tick} from 'svelte';
  import {Copy, Eye, RotateCcw, Send} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText} from '../../lib/notify.svelte';
  import {RetentionIntent} from '../../lib/model.js';
  import type {SettingsState, Uncertain} from './state.svelte';

  let {ctl, uncertain}: {ctl: SettingsState; uncertain: Uncertain} = $props();

  const transfer = $derived(uncertain.attempt.kind === 'transfer');
  const body = $derived(uncertain.attempt.body);
  const title = $derived(uncertain.stage === 'pending'
    ? transfer ? 'Ownership transfer outcome unconfirmed' : 'Settings request not confirmed'
    : uncertain.stage === 'discarded' ? 'Local transfer copy discarded'
      : transfer ? 'Original transfer retained as a local copy' : 'Local request tracking forgotten');
  const label = $derived(uncertain.stage === 'pending'
    ? transfer ? 'Original ownership transfer' : 'Original settings request'
    : transfer ? 'Local transfer copy' : 'Forgotten settings request');

  // Keep keyboard users in the box when its controls change underneath them.
  let heading: HTMLHeadingElement | null = $state(null);
  $effect(() => {
    void uncertain.stage;
    void tick().then(() => {
      if (!document.activeElement || document.activeElement === document.body) heading?.focus();
    });
  });

  function retry() {
    void app.run(() => ctl.changeRetention(uncertain.attempt as RetentionIntent), 'settings-retry');
  }
</script>

<Card class="border-warning/40 bg-warning/[0.04]">
  <CardContent class="space-y-4">
    <div class="space-y-1.5" role="status">
      <h2 bind:this={heading} tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{title}</h2>
      {#if uncertain.stage === 'pending'}
        {#if transfer}
          <p class="text-sm">{uncertain.error}</p>
          <p class="text-sm text-muted-foreground">There is no operation ID or safe retry. The transfer may have completed and revoked your former-owner keys. Read current ownership if this key still has access, or ask the intended new owner, an instance admin or the host operator to inspect. An observation does not prove which request changed ownership.</p>
        {:else}
          <p class="text-sm">{uncertain.error}</p>
          <p class="text-sm text-muted-foreground">
            {uncertain.attempt.kind === 'lifecycle'
              ? 'This endpoint has no operation ID or safe retry. Nothing will resend it automatically. Reading current state cannot prove which request changed it. The original request may still complete later.'
              : 'The original policy change, UUID, base revision and proof are retained. Nothing retries automatically. Compare the current policy before deliberately reconciling it.'}
          </p>
        {/if}
      {:else if uncertain.stage === 'forgotten'}
        <p class="text-sm text-muted-foreground">
          {transfer
            ? 'The server action remains uncancelled. Ask an authorized key holder to inspect current ownership before deciding on any further action.'
            : 'The original may still execute. Copy it if needed, then explicitly discard and reload current settings before another action.'}
        </p>
      {:else}
        <p class="text-sm text-muted-foreground">No server cancellation or current ownership read is implied.</p>
      {/if}
    </div>

    {#if uncertain.stage !== 'discarded'}
      <Field id="settings-original-request" label={label}>
        <Textarea id="settings-original-request" readonly value={body} rows={5} class="font-mono text-xs" />
      </Field>
    {/if}
    {#if uncertain.observation}<p class="text-sm" role="status">{uncertain.observation}</p>{/if}

    <div class="flex flex-wrap gap-2">
      {#if uncertain.stage === 'pending'}
        {#if uncertain.attempt.kind === 'retention'}
          <Button busy={app.working('settings-retry')} onclick={retry}><Send /> Check or retry exact policy change</Button>
        {/if}
        <Button variant="outline" onclick={() => copyText(body)}><Copy /> {transfer ? 'Copy original transfer' : 'Copy original settings request'}</Button>
        <Button variant="outline" busy={app.working('settings-compare')} onclick={() => app.run(() => ctl.compare(), 'settings-compare')}>
          <Eye /> {transfer ? 'Read current ownership' : 'Read current state'}
        </Button>
        <Button variant="ghost" busy={app.working('settings-forget')} onclick={() => app.run(() => ctl.forget(), 'settings-forget')}>
          {transfer ? 'Forget transfer locally' : 'Forget local settings attempt'}
        </Button>
      {:else if uncertain.stage === 'forgotten'}
        <Button variant="outline" onclick={() => copyText(body)}><Copy /> {transfer ? 'Copy transfer' : 'Copy original settings request'}</Button>
        {#if transfer}
          <Button variant="ghost" busy={app.working('settings-discard')} onclick={() => app.run(() => ctl.discardTransferCopy(), 'settings-discard')}>Discard local transfer copy</Button>
        {:else}
          <Button variant="ghost" busy={app.working('settings-reload')}
            onclick={() => app.run(() => ctl.reload(uncertain.attempt.kind === 'lifecycle'), 'settings-reload')}><RotateCcw /> Discard configuration and reload</Button>
        {/if}
      {:else}
        <Button variant="outline" busy={app.working('settings-reload')} onclick={() => app.run(() => ctl.refresh(), 'settings-reload')}><RotateCcw /> Read current workspace</Button>
      {/if}
    </div>
  </CardContent>
</Card>
