<script lang="ts">
  // Archived entry retention. Timed cleanup is enabled only from manual
  // retention with an explicit duration and confirmation; a timed policy must be
  // disabled before another duration is chosen. Changes carry the base revision.
  import {Copy, RotateCcw, Timer, TimerOff} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText, stamp} from '../../lib/notify.svelte';
  import type {Workspace} from '../../lib/types';
  import type {SettingsState} from './state.svelte';

  let {ctl, workspace}: {ctl: SettingsState; workspace: Workspace} = $props();

  const units = [{value: '86400', label: 'Days'}, {value: '3600', label: 'Hours'}, {value: '1', label: 'Seconds'}];
  const policy = $derived(ctl.policy);

  function enable(event: SubmitEvent) {
    event.preventDefault();
    void app.run(() => ctl.enableRetention(), 'retention-enable');
  }
</script>

<Card>
  <CardHeader title="Archived entry retention"
    description="How long individually archived entries are kept before permanent cleanup.">
    {#snippet action()}
      {#if policy}
        <Badge tone={policy.retention_seconds === null ? 'neutral' : 'info'}>{policy.retention_seconds === null ? 'Manual' : 'Timed cleanup'}</Badge>
      {/if}
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    {#if ctl.retentionReceipt}
      <Alert tone="success" title="Retention change acknowledged" live>{ctl.retentionReceipt}</Alert>
    {/if}

    {#if ctl.loading}
      <div class="space-y-2" aria-hidden="true"><Skeleton class="h-4 w-3/4" /><Skeleton class="h-4 w-1/2" /></div>
      <p class="sr-only" role="status">Loading current policy</p>
    {:else if !policy}
      <Alert tone="warning" title="Retention policy unavailable" live>
        <p>{ctl.policyError}</p>
        <p>No manual or timed policy was assumed. Lifecycle actions remain subject to their own permissions.</p>
      </Alert>
      <Button variant="outline" disabled={ctl.frozen} busy={app.working('retention-reload')}
        onclick={() => app.run(() => ctl.reload(), 'retention-reload')}><RotateCcw /> Read current settings</Button>
      {#if app.dirty && app.draft}
        <div class="space-y-3 rounded-lg border p-4">
          <Field id="settings-local-draft" label="Local configuration draft">
            <Textarea id="settings-local-draft" readonly value={app.draft} rows={6} class="font-mono text-xs" />
          </Field>
          <div class="flex flex-wrap gap-2">
            <Button variant="outline" onclick={() => copyText(app.draft)}><Copy /> Copy configuration draft</Button>
            <Button variant="ghost" onclick={() => app.run(() => ctl.discardDraft(), 'retention-discard')}>Discard local configuration draft</Button>
          </div>
        </div>
      {/if}
    {:else}
      <div class="space-y-1.5 text-sm">
        <p>
          {#if policy.retention_seconds === null}
            Manual retention: keep individually archived entries until privileged manual purge.
          {:else}
            Timed cleanup: individually archived entries become due after {policy.retention_seconds} seconds.{workspace.archived ? ' Paused while this workspace is archived.' : ''}
          {/if}
        </p>
        <p class="text-[13px] text-muted-foreground">
          Policy revision {policy.revision} · {policy.changed_at === null ? 'Default manual policy' : `Changed ${stamp(policy.changed_at)}`}
        </p>
      </div>
      <p class="text-sm text-muted-foreground">
        Timed cleanup permanently removes due archived entries and their revision history, retaining body-free operation and audit proofs. Backup retention is separate. The policy belongs to this workspace; revoking an admin key does not disable it.
      </p>

      {#if !app.writable()}
        <p class="text-sm text-muted-foreground">
          {workspace.archived ? 'Reactivate the workspace before changing retention.' : 'Use a workspace owner or admin key to change this policy.'}
        </p>
      {:else if policy.retention_seconds !== null}
        <p class="text-sm">To choose another duration, first disable timed cleanup, then explicitly enable it again. Existing individual archives receive fresh full windows on re-enable.</p>
        <Button variant="outline" disabled={ctl.frozen} busy={app.working('retention-disable')}
          onclick={() => app.run(() => ctl.disableRetention(), 'retention-disable')}><TimerOff /> Disable timed cleanup</Button>
      {:else}
        <form class="space-y-4 border-t pt-4" onsubmit={enable} oninput={() => ctl.retentionDraft()}>
          <p class="text-sm">
            Enabling starts a fresh full window for every existing individually archived entry. Future entry archives get that same window. Workspace archive pauses the remaining time. No duration is selected by default.
          </p>
          <div class="grid gap-4 sm:grid-cols-[minmax(0,12rem)_minmax(0,10rem)]">
            <Field id="retention-duration" label="Retention duration">
              <Input id="retention-duration" type="number" min="1" step="1" required inputmode="numeric"
                disabled={ctl.frozen} bind:value={ctl.retention.duration} />
            </Field>
            <Field id="retention-unit" label="Duration unit">
              <Select id="retention-unit" options={units} disabled={ctl.frozen} bind:value={ctl.retention.unit} />
            </Field>
          </div>
          <label class="flex items-start gap-3 text-sm">
            <input type="checkbox" required class="mt-0.5 size-4 shrink-0 accent-primary" disabled={ctl.frozen}
              bind:checked={ctl.retention.confirmed} />
            <span>I confirm permanent timed deletion of due archived entries and their revision history.</span>
          </label>
          <div class="flex flex-wrap gap-2">
            <Button type="submit" disabled={ctl.frozen} busy={app.working('retention-enable')}><Timer /> Enable timed cleanup</Button>
            <Button variant="ghost" disabled={ctl.frozen} busy={app.working('retention-reload')}
              onclick={() => app.run(() => ctl.reload(), 'retention-reload')}>Discard configuration and reload</Button>
          </div>
        </form>
      {/if}
    {/if}
  </CardContent>
</Card>
