<script lang="ts">
  // Entry detail: read, revise, append, archive, or create a new entry.
  // The entry fields and the separate append note are tracked apart; the
  // recovery copy (app.draft) is recomputed from both on every change.
  import {untrack} from 'svelte';
  import {Archive, History, DatabaseZap, RotateCcw, Send} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import Separator from '../../lib/ui/Separator.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {exactRevision} from '../../lib/model.js';
  import PendingWrite from './PendingWrite.svelte';
  import EntryMeta from './EntryMeta.svelte';
  import EntryHistory from './EntryHistory.svelte';
  import NativeHistory from './NativeHistory.svelte';
  import {focusId, type EntryReceipt, type HeldIntent, type Snapshot} from './content';
  import {actionOf, sendEntryWrite, startEntryWrite, type EntryChange} from './entryWrite';

  let {snapshot, bookId, onwritten, onreload}: {
    snapshot: Snapshot | null; bookId: string;
    onwritten: (action: EntryChange['action'], receipt: EntryReceipt) => Promise<void>;
    onreload: () => Promise<void>;
  } = $props();

  const initial = untrack(() => snapshot);
  const entry = initial?.entry ?? null;
  const original = {
    description: initial?.content.description ?? '',
    labels: (initial?.content.labels ?? []).join('\n'),
    text: initial?.content.text ?? '',
  };
  const writable = app.writable();

  let name = $state('');
  let description = $state(original.description);
  let labels = $state(original.labels);
  let text = $state(original.text);
  let note = $state('');
  let noteOpen = $state(false);
  let unconfirmed = $state.raw<{intent: HeldIntent; error: string} | null>(null);
  let forgotten = $state(false);
  let historyShown = $state(0);
  let sourceShown = $state(0);

  const frozen = $derived(!writable || app.pending !== null);
  const entryChanged = $derived(entry
    ? description !== original.description || labels !== original.labels || text !== original.text
    : Boolean(name || description || labels || text));

  // Recompute the recovery copy from both the entry fields and the note.
  function sync() {
    const hasNote = note.trim() !== '';
    if (!entryChanged && !hasNote) { app.clean(); return; }
    app.setDraft(JSON.stringify({
      ...(entryChanged ? {entry: {name: entry?.name ?? name, description, labels, text}} : {}),
      ...(hasNote ? {append_note: note} : {}),
    }, null, 2));
  }

  async function write(change: EntryChange) {
    const intent = await startEntryWrite(change);
    await send(intent);
  }

  async function send(intent: HeldIntent) {
    const outcome = await sendEntryWrite(intent);
    if (!outcome) return;
    if (!outcome.ok) {
      unconfirmed = {intent, error: outcome.message};
      notice(`${outcome.message} The original write is kept in the editor.`, 'warning');
      return;
    }
    unconfirmed = null;
    forgotten = false;
    app.clean();
    notice(`Write confirmed at revision ${outcome.receipt.revision}.`, 'success');
    await onwritten(actionOf(intent), outcome.receipt);
  }

  function save(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (note.trim()) throw new Error('Append or discard your separate note before saving entry edits.');
      const content = {description, labels: labels.split('\n').filter(Boolean), text};
      await write(entry
        ? {action: 'revise', entry_id: entry.id, expected_revision: exactRevision(entry.revision), content}
        : {action: 'create', book_id: bookId, name, content});
    }, 'entry-save');
  }

  function append(event: SubmitEvent) {
    event.preventDefault();
    if (!entry) return;
    void app.run(async () => {
      // A full edit and a separate append must not silently discard each other.
      if (entryChanged) throw new Error('Save or discard entry edits before appending.');
      await write({action: 'append', entry_id: entry.id, expected_revision: exactRevision(entry.revision), text: note});
    }, 'entry-append');
  }

  function archive() {
    if (!entry) return;
    void app.run(async () => {
      if (!app.mayLeave()) return;
      const ok = await confirmAction({title: 'Archive this entry?', body: 'Its body and history become unavailable until it is restored.', confirm: 'Archive entry', destructive: true});
      if (ok) await write({action: 'archive', entry_id: entry.id, expected_revision: exactRevision(entry.revision)});
    }, 'entry-archive');
  }

  function discard() {
    void app.run(async () => {
      if (app.pending) { notice('Resolve or explicitly forget the uncertain write first.', 'warning'); return; }
      if (app.dirty) {
        const ok = await confirmAction(entry
          ? {title: 'Discard edits and reload?', body: 'Unsaved edits and any append note are discarded, then the latest revision is read.', confirm: 'Discard and reload', destructive: true}
          : {title: 'Discard this new entry?', body: 'The unsaved entry is discarded. Nothing was sent to the server.', confirm: 'Discard', destructive: true});
        if (!ok) return;
      }
      // Keep the draft until the latest snapshot is successfully read.
      if (entry) await onreload();
      else { name = ''; description = ''; labels = ''; text = ''; note = ''; }
      forgotten = false;
      app.clean();
    }, 'entry-reload');
  }

  async function discardNote() {
    if (app.pending) { notice('Resolve or explicitly forget the uncertain write first.', 'warning'); return; }
    if (note && !(await confirmAction({title: 'Discard the append note?', body: 'The unsaved note is removed. Entry edits are kept.', confirm: 'Discard note', destructive: true}))) return;
    note = '';
    noteOpen = false;
    sync();
    void focusId('entry-note-summary');
  }

  function retry() {
    const held = unconfirmed;
    if (held) void app.run(() => send(held.intent), 'entry-pending-retry');
  }

  async function forget() {
    const held = unconfirmed;
    if (!held) return;
    const ok = await confirmAction({title: 'Forget this local attempt?', body: 'It may already have completed, and forgetting does not cancel it. Copy the original request first if you need it.', confirm: 'Forget attempt', destructive: true});
    if (!ok) return;
    app.pending = null;
    unconfirmed = null;
    forgotten = true;
    // Keep local text until an explicit discard or reload.
    sync();
    if (!app.dirty) app.setDraft(held.intent.body);
    void focusId('entry-forgotten');
  }
</script>

<article class="space-y-5" aria-labelledby="context-detail-title">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div class="min-w-0 space-y-1">
      <h2 id="context-detail-title" tabindex="-1" class="text-lg leading-tight font-semibold tracking-tight break-words outline-none">{entry ? entry.name : 'New entry'}</h2>
      {#if !entry}<p class="text-sm text-muted-foreground">Created in the selected book.</p>{/if}
    </div>
    {#if entry}<Badge tone="outline" class="tabular">Revision {entry.revision}</Badge>{/if}
  </div>

  {#if entry}<EntryMeta {entry} />{/if}
  {#if !writable}<Alert>Read-only with this key or workspace state.</Alert>{/if}
  {#if unconfirmed}
    <PendingWrite id="entry-pending" intent={unconfirmed.intent} error={unconfirmed.error} onretry={retry} onforget={forget} />
  {/if}
  {#if forgotten}
    <p id="entry-forgotten" tabindex="-1" class="rounded-md border border-warning/35 bg-warning/[0.06] px-3 py-2 text-sm outline-none">
      Local attempt forgotten. It may still have completed. Copy your text before you discard or reload the editor.
    </p>
  {/if}

  <form class="grid gap-4" onsubmit={save} oninput={sync}>
    {#if !entry}
      <Field id="entry-name" label="Entry name">
        <Input id="entry-name" bind:value={name} required readonly={frozen} autocomplete="off" />
      </Field>
    {/if}
    <Field id="entry-description" label="Description">
      <Input id="entry-description" bind:value={description} readonly={frozen} autocomplete="off" />
    </Field>
    <Field id="entry-labels" label="Labels" hint="One label per line.">
      <Textarea id="entry-labels" bind:value={labels} readonly={frozen} rows={2} class="min-h-0" aria-describedby="entry-labels-hint" />
    </Field>
    <Field id="entry-text" label="Entry text">
      <Textarea id="entry-text" bind:value={text} readonly={frozen} rows={14} class="font-mono text-[13px] leading-relaxed" />
    </Field>
    {#if writable}
      <div class="flex flex-wrap justify-end gap-2">
        <Button variant="outline" busy={app.working('entry-reload')} onclick={discard}><RotateCcw /> {entry ? 'Discard edits and reload' : 'Discard draft'}</Button>
        <Button type="submit" busy={app.working('entry-save')}>{entry ? 'Save new revision' : 'Create entry'}</Button>
      </div>
    {/if}
  </form>

  {#if entry && writable}
    <Separator />
    <details bind:open={noteOpen} class="group rounded-lg border">
      <summary id="entry-note-summary" class="cursor-pointer rounded-lg px-4 py-3 text-sm font-medium outline-none select-none focus-visible:ring-2 focus-visible:ring-ring">
        Append a note {#if note.trim()}<span class="font-normal text-muted-foreground">(unsaved)</span>{/if}
      </summary>
      <form class="grid gap-3 px-4 pb-4" onsubmit={append} oninput={sync}>
        <Field id="entry-note" label="Contribution" hint="Appended as a new revision without changing the fields above.">
          <Textarea id="entry-note" bind:value={note} required readonly={frozen} rows={4} aria-describedby="entry-note-hint" />
        </Field>
        <div class="flex flex-wrap justify-end gap-2">
          <Button variant="ghost" onclick={discardNote}>Discard note</Button>
          <Button type="submit" variant="secondary" busy={app.working('entry-append')}><Send /> Append note</Button>
        </div>
      </form>
    </details>
  {/if}

  {#if entry}
    <div class="flex flex-wrap items-center gap-2">
      <Button variant="outline" size="sm" onclick={() => (historyShown += 1)}><History /> Revision history</Button>
      <Button variant="outline" size="sm" onclick={() => (sourceShown += 1)}><DatabaseZap /> Source history</Button>
      {#if writable}
        <Button variant="ghost" size="sm" class="ml-auto text-destructive hover:text-destructive" busy={app.working('entry-archive')} onclick={archive}><Archive /> Archive entry</Button>
      {/if}
    </div>
    {#if historyShown}
      {#key historyShown}<EntryHistory entryId={entry.id} />{/key}
    {/if}
    {#if sourceShown}
      {#key sourceShown}<NativeHistory entryId={entry.id} />{/key}
    {/if}
  {/if}
</article>
