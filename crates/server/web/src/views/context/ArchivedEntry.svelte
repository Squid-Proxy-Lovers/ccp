<script lang="ts">
  // An individually archived entry: summary metadata, restore, and admin purge.
  import {ArchiveRestore} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {canManageInvitations, exactRevision} from '../../lib/model.js';
  import EntryMeta from './EntryMeta.svelte';
  import PendingWrite from './PendingWrite.svelte';
  import PurgeForm from './PurgeForm.svelte';
  import {focusId, type EntryReceipt, type EntrySummary, type HeldIntent} from './content';
  import {actionOf, sendEntryWrite, startEntryWrite, type EntryChange} from './entryWrite';
  import type {AdminIntent, ArchiveOutcome} from './archiveWrite';

  let {entry, onwritten, onarchive}: {
    entry: EntrySummary;
    onwritten: (action: EntryChange['action'], receipt: EntryReceipt) => Promise<void>;
    onarchive: (intent: AdminIntent, outcome: ArchiveOutcome) => void;
  } = $props();

  const writable = app.writable();
  const scoped = Boolean(app.session && canManageInvitations(app.session, app.workspace));
  let unconfirmed = $state.raw<{intent: HeldIntent; error: string} | null>(null);
  let forgotten = $state(false);

  async function send(intent: HeldIntent) {
    const outcome = await sendEntryWrite(intent);
    if (!outcome) return;
    if (!outcome.ok) {
      unconfirmed = {intent, error: outcome.message};
      notice(`${outcome.message} The original write is retained below.`, 'warning');
      return;
    }
    unconfirmed = null;
    app.clean();
    notice(`Write confirmed at revision ${outcome.receipt.revision}.`, 'success');
    await onwritten(actionOf(intent), outcome.receipt);
  }

  function restore() {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      const ok = await confirmAction({title: 'Restore this entry?', body: 'A name collision with an active entry will be refused.', confirm: 'Restore entry'});
      if (!ok) return;
      await send(await startEntryWrite({action: 'restore', entry_id: entry.id, expected_revision: exactRevision(entry.revision)}));
    }, 'entry-restore');
  }

  async function forget() {
    const held = unconfirmed;
    if (!held) return;
    const ok = await confirmAction({title: 'Forget this local attempt?', body: 'It may already have completed, and forgetting does not cancel it. Copy the original request first if you need it.', confirm: 'Forget attempt', destructive: true});
    if (!ok) return;
    app.pending = null;
    unconfirmed = null;
    forgotten = true;
    app.setDraft(held.intent.body);
    void focusId('archived-forgotten');
  }

  async function discardCopy() {
    if (!(await confirmAction({title: 'Discard the local copy?', body: 'This does not cancel any server request.', confirm: 'Discard copy', destructive: true}))) return;
    app.clean();
    forgotten = false;
    void focusId('context-detail-title');
  }
</script>

<article class="space-y-5" aria-labelledby="context-detail-title">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <h2 id="context-detail-title" tabindex="-1" class="text-lg leading-tight font-semibold tracking-tight break-words outline-none">{entry.name}</h2>
    <div class="flex gap-2"><Badge tone="warning">Archived</Badge><Badge tone="outline" class="tabular">Revision {entry.revision}</Badge></div>
  </div>
  <EntryMeta {entry} />
  <Alert>Archived entry. Its body and history are unavailable until it is restored.</Alert>
  {#if unconfirmed}
    <PendingWrite id="restore-pending" intent={unconfirmed.intent} error={unconfirmed.error} onforget={forget}
      onretry={() => { const held = unconfirmed; if (held) void app.run(() => send(held.intent), 'restore-pending-retry'); }} />
  {/if}
  {#if forgotten}
    <div class="flex flex-wrap items-center gap-3 rounded-md border border-warning/35 bg-warning/[0.06] px-3 py-2">
      <p id="archived-forgotten" tabindex="-1" class="min-w-0 flex-1 text-sm outline-none">
        Local attempt forgotten. It may still have completed. The original request stays in local recovery; read the entry lists before another action.
      </p>
      <Button variant="ghost" size="sm" onclick={discardCopy}>Discard local copy</Button>
    </div>
  {/if}
  {#if writable}
    <Button busy={app.working('entry-restore')} onclick={restore}><ArchiveRestore /> Restore entry</Button>
  {/if}
  {#if scoped && entry.archived}<PurgeForm {entry} {onarchive} />{/if}
</article>
