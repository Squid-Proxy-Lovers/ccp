<script lang="ts">
  // Owner-only ownership transfer to another enabled workspace administrator.
  // The request has no operation ID and is never resent; on success every
  // former-owner key, including this login, loses authority.
  import {ChevronLeft, ChevronRight, UserRoundCog} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import SkeletonTable from '../../lib/ui/skeleton/SkeletonTable.svelte';
  import {table} from '../../lib/ui/table';
  import {app} from '../../lib/app.svelte';
  import {canTransferOwnership} from '../../lib/model.js';
  import type {Session, Workspace} from '../../lib/types';
  import type {SettingsState} from './state.svelte';

  let {ctl, workspace, session}: {ctl: SettingsState; workspace: Workspace; session: Session} = $props();

  let delivered: HTMLInputElement | null = $state(null);
  $effect(() => { if (ctl.admin && delivered) delivered.focus(); });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(() => ctl.submitTransfer(), 'transfer');
  }
</script>

<Card>
  <CardHeader title="Transfer ownership"
    description="There is one owner at a time. Transfer to another enabled administrator of this workspace who already holds a workspace key." />
  <CardContent class="space-y-4">
    {#if workspace.archived}
      <p class="text-sm text-muted-foreground">Reactivate this workspace before transferring ownership.</p>
    {:else if ctl.admin}
      {@const admin = ctl.admin}
      <form class="space-y-4" onsubmit={submit} oninput={() => ctl.transferDraft()}>
        <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[9rem_1fr]">
          <dt class="text-muted-foreground">Workspace</dt>
          <dd class="min-w-0 break-all">{workspace.name} · <code class="font-mono text-[13px]">{workspace.id}</code></dd>
          <dt class="text-muted-foreground">Current owner</dt>
          <dd class="min-w-0 break-all"><code class="font-mono text-[13px]">{session.identity.principal_id}</code></dd>
          <dt class="text-muted-foreground">New owner</dt>
          <dd class="min-w-0 break-all">{admin.name} · <code class="font-mono text-[13px]">{admin.id}</code></dd>
        </dl>
        <Alert tone="warning" title="Your keys lose authority">
          The recipient’s existing workspace key gains owner powers. Every workspace key of the former owner is revoked, including this login key. Unrelated roles, keys and stored content are preserved. No new key secret is returned.
        </Alert>
        <p class="text-sm text-muted-foreground">
          The server requires an enabled current workspace administrator with a live scoped key record. A record does not prove delivery; verify privately that the recipient can use their key before transferring.
        </p>
        <label class="flex items-start gap-3 text-sm">
          <input bind:this={delivered} type="checkbox" required class="mt-0.5 size-4 shrink-0 accent-primary"
            disabled={ctl.frozen} bind:checked={ctl.transfer.delivered} />
          <span>I have delivered this administrator’s workspace key and confirmed that they can use it.</span>
        </label>
        <Field id="transfer-target" label="Confirm new owner principal ID" hint="Type the new owner’s exact principal ID.">
          <Input id="transfer-target" required autocomplete="off" spellcheck={false} aria-describedby="transfer-target-hint"
            class="font-mono" disabled={ctl.frozen} bind:value={ctl.transfer.target} />
        </Field>
        <div class="flex flex-wrap gap-2">
          <Button type="submit" variant="destructive" disabled={ctl.frozen} busy={app.working('transfer')}>Confirm ownership transfer</Button>
          <Button variant="ghost" disabled={ctl.frozen} onclick={() => app.run(() => ctl.discardTransfer(), 'transfer-discard')}>Discard transfer draft</Button>
        </div>
      </form>
    {:else if ctl.rosterLoading && !ctl.roster.length}
      <div class="-mx-5 border-y"><Delayed><SkeletonTable rows={2} columns={['w-48', 'w-16', 'w-32']} label="Loading administrators" /></Delayed></div>
    {:else if ctl.rosterError}
      <Alert tone="warning" title="Administrators unavailable" live>{ctl.rosterError}</Alert>
      <Button variant="outline" disabled={ctl.frozen} busy={app.working('roster')}
        onclick={() => app.run(() => ctl.rosterPage(ctl.rosterAfter), 'roster')}>Read administrators</Button>
    {:else}
      <p class="text-sm text-muted-foreground">
        A principal ID identifies the registered administrator; names and shared keys do not prove which person is using them.
      </p>
      <div class={['-mx-5 border-y', table.wrap]}>
        <table class={table.table}>
          <caption class="sr-only">Workspace administrators</caption>
          <thead class={table.head}>
            <tr><th scope="col" class={table.headCell}>Principal</th><th scope="col" class={table.headCell}>Role</th>
              <th scope="col" class={table.headCell}><span class="sr-only">Actions</span></th></tr>
          </thead>
          <tbody class={table.body}>
            {#each ctl.roster as admin (admin.id)}
              <tr class={table.row}>
                <td class={table.cell}>
                  <div class="font-medium">{admin.name}</div>
                  <code class="font-mono text-xs break-all text-muted-foreground">{admin.id}</code>
                </td>
                <td class={table.cell}>
                  <div class="flex flex-wrap gap-1.5">
                    <Badge tone={admin.is_owner ? 'info' : 'neutral'}>{admin.is_owner ? 'Owner' : 'Administrator'}</Badge>
                    {#if !admin.enabled}<Badge tone="warning">Principal disabled</Badge>{/if}
                  </div>
                </td>
                <td class={[table.cell, 'text-right']}>
                  {#if canTransferOwnership(session, workspace, admin)}
                    <Button variant="outline" size="sm" disabled={ctl.frozen}
                      onclick={() => app.run(() => ctl.prepareTransfer(admin), 'transfer-prepare')}>
                      <UserRoundCog /> Transfer ownership<span class="sr-only"> to {admin.name}</span>
                    </Button>
                  {/if}
                </td>
              </tr>
            {:else}
              <tr><td colspan="3" class={[table.cell, 'text-muted-foreground']}>No administrators on this page.</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if ctl.rosterNext || ctl.rosterAfter}
        <div class="flex justify-end gap-2">
          <Button variant="outline" size="sm" disabled={ctl.frozen || !ctl.rosterAfter} busy={app.working('roster-first')}
            onclick={() => app.run(() => ctl.rosterPage(null), 'roster-first')}><ChevronLeft /> First administrators</Button>
          <Button variant="outline" size="sm" disabled={ctl.frozen || !ctl.rosterNext} busy={app.working('roster-next')}
            onclick={() => app.run(() => ctl.rosterPage(ctl.rosterNext), 'roster-next')}>Next administrators <ChevronRight /></Button>
        </div>
      {/if}
    {/if}
  </CardContent>
</Card>
