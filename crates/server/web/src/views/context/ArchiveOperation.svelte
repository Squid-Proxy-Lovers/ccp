<script lang="ts">
  // Outcome of a shelf transition or entry purge: a verified receipt, or an
  // unconfirmed original that may only be retried exactly, observed, or
  // explicitly forgotten (which never cancels it on the server).
  import {untrack} from 'svelte';
  import {Copy, Eye, RotateCcw, Trash2} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText, notice} from '../../lib/notify.svelte';
  import {shelfState} from '../../lib/model.js';
  import {PAGE_SIZE, contentPath, focusId, message} from './content';
  import {sendArchive, type AdminIntent, type ArchiveOutcome} from './archiveWrite';

  let {intent, outcome: initial, onconfirmed, onread, onreturn}: {
    intent: AdminIntent; outcome: ArchiveOutcome;
    onconfirmed: (outcome: Extract<ArchiveOutcome, {state: 'confirmed'}>) => Promise<void>;
    onread: () => Promise<void>; onreturn: () => Promise<void>;
  } = $props();

  type Phase = ArchiveOutcome | {state: 'forgotten'} | {state: 'discarded'};
  let phase = $state.raw<Phase>(untrack(() => initial));
  let observation = $state('');
  const title = $derived({
    confirmed: 'Archive operation acknowledged',
    unconfirmed: 'Archive operation outcome unconfirmed',
    forgotten: 'Original archive operation kept as a local copy',
    discarded: 'Local archive work discarded',
  }[phase.state]);

  function retry() {
    void app.run(async () => {
      const next = await sendArchive(intent);
      if (!next) return;
      phase = next;
      observation = '';
      void focusId('context-detail-title');
      if (next.state === 'confirmed') await onconfirmed(next);
    }, 'archive-retry');
  }

  function observe() {
    void app.run(async () => {
      const data = await app.request(contentPath(intent.action === 'shelf' ? 'shelves/state' : 'entries/list'), intent.action === 'shelf'
        ? {workspace_id: intent.workspace_id, node_id: intent.target_id}
        : {workspace_id: intent.workspace_id, book_id: intent.book_id, archived: true, limit: PAGE_SIZE});
      if (intent.action === 'shelf') shelfState(data, intent.workspace_id, intent.target_id);
      observation = JSON.stringify(data, null, 2);
    }, 'archive-observe');
  }

  async function forget() {
    if (!(await confirmAction({title: 'Forget this operation locally?', body: 'It is not cancelled and may have completed. Copy it first if you need it.', confirm: 'Forget locally', destructive: true}))) return;
    app.pending = null;
    app.setDraft(intent.body);
    phase = {state: 'forgotten'};
    void focusId('context-detail-title');
  }

  async function discard() {
    if (!(await confirmAction({title: 'Discard the local copy?', body: 'This does not cancel any server operation.', confirm: 'Discard copy', destructive: true}))) return;
    app.clean();
    phase = {state: 'discarded'};
    void focusId('context-detail-title');
  }

  function read() {
    const confirmed = phase.state === 'confirmed' ? phase.text : '';
    void app.run(async () => {
      try { await onread(); }
      catch (error) {
        if (app.locked) app.lock(`${confirmed} Current metadata is unavailable.`);
        else notice(`${confirmed} Current metadata read failed: ${message(error)}`, 'warning');
      }
    }, 'archive-read');
  }
</script>

<article class="space-y-4" aria-labelledby="context-detail-title">
  <h2 id="context-detail-title" tabindex="-1" class="text-lg leading-tight font-semibold tracking-tight outline-none">{title}</h2>

  {#if phase.state === 'confirmed'}
    <Alert tone="success" live>
      <p class="break-all">{phase.text}</p>
      <p>This receipt records the original operation, not current visibility or a guarantee of forensic erasure.</p>
    </Alert>
    <Button variant="outline" busy={app.working('archive-read')} onclick={read}><Eye /> Read current metadata</Button>
  {:else if phase.state === 'unconfirmed'}
    <Alert tone="warning" live>
      <div class="space-y-2">
        <p>{phase.error}</p>
        <p>No automatic retry was sent. Keep the exact original operation and key. A deliberate exact retry returns its historical receipt without reapplying. An observation cannot attribute this request. A conflict needs a separate decision after discarding this original; never replace its operation ID or revision.</p>
      </div>
    </Alert>
    <Field id="archive-operation" label="Original archive operation">
      <Textarea id="archive-operation" readonly value={intent.body} rows={5} class="font-mono text-xs" />
    </Field>
    <div class="flex flex-wrap gap-2">
      <Button busy={app.working('archive-retry')} onclick={retry}><RotateCcw /> Check or retry exact operation</Button>
      <Button variant="outline" onclick={() => copyText(intent.body)}><Copy /> Copy original operation</Button>
      <Button variant="outline" busy={app.working('archive-observe')} onclick={observe}><Eye /> Read separate observation</Button>
      <Button variant="ghost" onclick={forget}><Trash2 /> Forget locally</Button>
    </div>
    {#if observation}
      <div class="space-y-2" aria-live="polite">
        <p class="text-sm text-muted-foreground">Separate current observation only; not attribution or cancellation. Entry lists show one page only.</p>
        <Field id="archive-observation" label="Current observation">
          <Textarea id="archive-observation" readonly value={observation} rows={8} class="font-mono text-xs" />
        </Field>
      </div>
    {/if}
  {:else if phase.state === 'forgotten'}
    <p class="text-sm text-muted-foreground">The operation was not cancelled and may have completed. This copy stays in local recovery until you discard it.</p>
    <Field id="archive-local-copy" label="Local archive operation copy">
      <Textarea id="archive-local-copy" readonly value={intent.body} rows={5} class="font-mono text-xs" />
    </Field>
    <div class="flex flex-wrap gap-2">
      <Button variant="outline" onclick={() => copyText(intent.body)}><Copy /> Copy archive operation</Button>
      <Button variant="ghost" onclick={discard}><Trash2 /> Discard local copy</Button>
    </div>
  {:else}
    <p class="text-sm text-muted-foreground">No server cancellation or successful read is implied.</p>
    <Button variant="outline" busy={app.working('archive-return')} onclick={() => app.run(onreturn, 'archive-return')}>Return to context browser</Button>
  {/if}
</article>
