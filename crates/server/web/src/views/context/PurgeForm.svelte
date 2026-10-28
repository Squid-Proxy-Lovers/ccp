<script lang="ts">
  // Irreversible purge of one individually archived entry: typed ID, explicit
  // consent and a final confirmation, bound to the entry's exact revision.
  import {Trash2} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {ContentAdminIntent} from '../../lib/model.js';
  import {focusId, type EntrySummary} from './content';
  import {sendArchive, type AdminIntent, type ArchiveOutcome} from './archiveWrite';

  let {entry, onarchive}: {entry: EntrySummary; onarchive: (intent: AdminIntent, outcome: ArchiveOutcome) => void} = $props();
  let open = $state(false);
  let typed = $state('');
  let consent = $state(false);

  function sync() {
    if (!typed && !consent) { app.clean(); return; }
    app.setDraft(JSON.stringify({entry_id: entry.id, expected_revision: entry.revision, typed_id: typed, confirm_purge: consent}));
  }

  async function discard() {
    if (!(await confirmAction({title: 'Discard this purge draft?', body: 'Nothing was sent to the server.', confirm: 'Discard draft'}))) return;
    typed = '';
    consent = false;
    open = false;
    app.clean();
    void focusId('purge-summary');
  }

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original archive operation first.');
      if (!consent) throw new Error('Explicitly confirm irreversible entry purge.');
      const intent = new ContentAdminIntent(app.session, app.workspace, entry, 'purge', typed) as unknown as AdminIntent;
      const ok = await confirmAction({
        title: 'Permanently purge this entry?',
        body: `Entry ${entry.id} in workspace ${entry.workspace_id} at revision ${entry.revision}. All payload and revision rows are removed. Backups are separate.`,
        confirm: 'Purge permanently', destructive: true,
      });
      if (!ok) return;
      const outcome = await sendArchive(intent);
      if (outcome) onarchive(intent, outcome);
    }, 'entry-purge');
  }
</script>

<details bind:open class="rounded-lg border border-destructive/35">
  <summary id="purge-summary" class="cursor-pointer rounded-lg px-4 py-3 text-sm font-medium text-destructive outline-none select-none focus-visible:ring-[3px] focus-visible:ring-ring/50">Permanently purge archived entry</summary>
  <form class="grid gap-4 px-4 pb-4" onsubmit={submit} oninput={sync} onchange={sync}>
    <p class="text-sm text-muted-foreground">
      Permanently removes entry <span class="font-mono text-xs break-all text-foreground">{entry.id}</span> at revision {entry.revision}, including every payload and revision row. It cannot be restored. Retained operation proofs and audit records remain. Separate backups, SQLite free pages and journals are not securely erased. Purge is allowed while the workspace is archived.
    </p>
    <Field id="purge-typed" label="Type the archived entry ID to confirm">
      <Input id="purge-typed" bind:value={typed} required autocomplete="off" spellcheck="false" class="font-mono text-xs" />
    </Field>
    <label class="flex items-start gap-2 text-sm">
      <input type="checkbox" bind:checked={consent} required class="mt-0.5 size-4 accent-destructive" />
      <span>I understand that this entry and all its revisions cannot be restored.</span>
    </label>
    <div class="flex flex-wrap justify-end gap-2">
      <Button variant="ghost" onclick={discard}>Discard purge draft</Button>
      <Button type="submit" variant="destructive" busy={app.working('entry-purge')}><Trash2 /> Permanently purge entry</Button>
    </div>
  </form>
</details>
