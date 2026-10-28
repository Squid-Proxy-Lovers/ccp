<script lang="ts">
  // Workspace deletion tracking. Lives in page memory (`app.deletion`) across
  // sign-in changes, holds only body-free IDs and receipts, and only ever
  // reads status: it never resends a deletion. Renders nothing when idle.
  import {tick} from 'svelte';
  import {Copy, KeyRound, RefreshCw} from '@lucide/svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Card from '../lib/ui/Card.svelte';
  import CardContent from '../lib/ui/CardContent.svelte';
  import Field from '../lib/ui/Field.svelte';
  import Textarea from '../lib/ui/Textarea.svelte';
  import {confirmAction} from '../lib/ui/confirm.svelte';
  import {app} from '../lib/app.svelte';
  import {copyText, notice, stamp} from '../lib/notify.svelte';
  import {deletionTracking} from '../lib/model.js';
  import {inspectDeletion, tracked as current} from './settings/deletion';

  let {class: className = ''}: {class?: string} = $props();

  const tracked = $derived(current());
  const receipt = $derived(tracked?.receipt ?? null);
  const details = $derived(tracked ? deletionTracking(tracked.reference, receipt) as string : '');
  const canInspect = $derived(Boolean(tracked && app.session && !app.locked && app.session.identity.scope.kind === 'instance'
    && app.session.instance_id === tracked.reference.instance_id));
  const title = $derived(receipt
    ? receipt.completed_at === null ? 'Workspace deletion started' : 'Workspace cleanup complete'
    : 'Workspace deletion outcome unconfirmed');

  let heading: HTMLHeadingElement | null = $state(null);
  // When the control that opened or updated tracking disappears, land on the tracker.
  $effect(() => {
    if (!heading) return;
    void title;
    void tick().then(() => {
      if (!document.activeElement || document.activeElement === document.body) heading?.focus();
    });
  });

  async function forget() {
    const value = tracked;
    if (!value) return;
    if (!await confirmAction({title: 'Forget local deletion tracking?', confirm: 'Forget tracking', destructive: true,
      body: 'This does not cancel deletion or a delayed request. Copy the details first.'})) return;
    if (app.pending === value.reference) { app.pending = null; app.clean(); }
    app.deletion = null;
    notice('Local deletion tracking forgotten. Any server deletion continues. Read current state before another action.', 'warning');
    await tick();
    document.getElementById('page-title')?.focus();
  }

  function changeKey() {
    app.lock('Keep the original deletion tracking, then sign in with an instance-admin key. The server request is not cancelled.');
  }
</script>

{#if tracked}
  <Card class="{receipt?.completed_at != null ? 'border-success/40' : 'border-warning/40'} {className}">
    <CardContent class="space-y-4">
      <div class="flex flex-wrap items-start justify-between gap-3" role="status">
        <div class="min-w-0 space-y-1.5">
          <h2 bind:this={heading} tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{title}</h2>
          <p class="text-sm text-muted-foreground">
            {#if !receipt}
              The original request may already have committed or may still arrive. No request retries automatically. Missing or unavailable status does not prove failure.
            {:else if receipt.completed_at === null}
              Access ended at {stamp(receipt.started_at)}. Cleanup is pending and can take hours for a large workspace. Check manually for completion.
            {:else}
              Live workspace content and history were removed by {stamp(receipt.completed_at)}. Minimal security and deletion records remain; backup retention is separate.
            {/if}
          </p>
        </div>
        <Badge tone={!receipt ? 'warning' : receipt.completed_at === null ? 'info' : 'success'}>
          {!receipt ? 'Unconfirmed' : receipt.completed_at === null ? 'Cleanup pending' : 'Complete'}
        </Badge>
      </div>
      <Field id="deletion-tracking" label="Deletion tracking details">
        <Textarea id="deletion-tracking" readonly value={details} rows={8} class="font-mono text-xs" />
      </Field>
      <p class="text-[13px] text-muted-foreground">
        Tracking stays in this page’s memory across sign-in changes. Copy it before reloading or closing the page. It contains no admin key or session proof.
      </p>
      {#if !canInspect}
        <p class="text-sm">Sign in with a current instance-admin key on this instance to inspect this operation. This never resends the deletion.</p>
      {/if}
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" onclick={() => copyText(details)}><Copy /> Copy deletion tracking</Button>
        {#if canInspect}
          <Button variant="outline" busy={app.working('deletion-status')}
            onclick={() => app.run(() => tracked && inspectDeletion(tracked), 'deletion-status')}><RefreshCw /> Check deletion completion</Button>
        {:else if app.session && !app.locked}
          <Button variant="outline" onclick={changeKey}><KeyRound /> Change key to inspect deletion</Button>
        {/if}
        <Button variant="ghost" onclick={() => app.run(forget, 'deletion-forget')}>Forget local deletion tracking</Button>
      </div>
    </CardContent>
  </Card>
{/if}
