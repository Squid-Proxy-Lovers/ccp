<script lang="ts">
  // One guidance board. A save is bound to the loaded revision and carries an
  // operation ID; an unconfirmed save keeps the exact request and never retries
  // on its own. Saves go live immediately.
  import {Copy, GitCompare, RotateCcw, Save} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText, notice, stamp} from '../../lib/notify.svelte';
  import {GuidanceIntent, wire} from '../../lib/model.js';
  import type {Session} from '../../lib/types';
  import {MAX_BYTES, bytes, readGuidance, type Board, type Bundle} from './guidance';

  const SAVE = '/api/v1/admin/guidance/save';

  let {board, workspaceId, editable, note, onreplace}: {
    board: Board; workspaceId: string | null; editable: boolean; note: string; onreplace: (bundle: Bundle) => void;
  } = $props();

  // The editor starts from the loaded snapshot; a new snapshot remounts this component.
  // svelte-ignore state_referenced_locally
  let text = $state(board.text);
  let edited = $state(false);
  /** The original save whose outcome is unknown; mirrors `app.pending`. */
  let intent = $state.raw<GuidanceIntent | null>(null);
  let failure = $state('');
  let conflict = $state(false);
  let comparison = $state.raw<Board | null>(null);
  let forgotten = $state(false);
  let confirmedRevision = $state<number | null>(null);

  const global = $derived(board.scope.kind === 'global');
  const id = $derived(`guidance-${board.scope.kind}`);
  const size = $derived(bytes(text));
  const over = $derived(size > MAX_BYTES);
  const busyKey = $derived(`guidance:${board.scope.kind}`);

  function input(event: Event & {currentTarget: HTMLTextAreaElement}) {
    edited = true;
    app.setDraft(JSON.stringify({scope: board.scope, base_revision: board.revision, text: event.currentTarget.value}, null, 2));
  }

  function save(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve or explicitly forget the original save first.');
      if (!text.length && !await confirmAction({
        title: 'Clear this board?',
        body: 'Saving empty guidance clears the board. An accepted save goes live immediately.',
        confirm: 'Save empty guidance',
        destructive: true,
      })) return;
      const next = new GuidanceIntent(app.session as Session, board.scope, board.revision, text);
      await app.verify();
      app.pending = next;
      intent = next;
      await send(next);
    }, `${busyKey}:save`);
  }

  async function send(next: GuidanceIntent): Promise<void> {
    let revision: number;
    try {
      await app.verify();
      const request = next.request(app.session as Session);
      revision = next.receipt(await wire(SAVE, request.body, request.proof));
    } catch (error) {
      if (app.locked) return;
      const status = (error as {status?: number}).status;
      if (status === 401 || status === 403) { app.lock((error as Error).message); return; }
      failure = (error as Error).message;
      conflict = status === 409;
      notice(`${failure} Your draft and original save are kept.`, 'warning');
      return;
    }
    app.pending = null;
    app.clean();
    intent = null;
    failure = '';
    conflict = false;
    comparison = null;
    forgotten = false;
    edited = false;
    confirmedRevision = revision;
    try {
      const bundle = await readGuidance(workspaceId);
      const latest = JSON.parse(next.body).scope.kind === 'global' ? bundle.global : bundle.workspace;
      onreplace(bundle);
      notice(`Guidance save confirmed at revision ${revision}. Showing current revision ${latest?.revision}.`, 'success');
    } catch (error) {
      notice(`Guidance save confirmed at revision ${revision}. Refresh failed: ${(error as Error).message}`, 'warning');
    }
  }

  function reload() {
    void app.run(async () => {
      if (app.pending) { notice('Resolve or explicitly forget the original save before reloading.', 'warning'); return; }
      if (app.dirty && !await confirmAction({
        title: 'Discard this guidance draft?',
        body: 'This reads the current boards and drops your edits. Copy any text you want to keep first.',
        confirm: 'Discard and reload',
        destructive: true,
      })) return;
      // Keep the draft until the read succeeds; a failed read can lock safely.
      const bundle = await readGuidance(workspaceId);
      app.clean();
      onreplace(bundle);
      notice('Loaded current guidance.');
    }, `${busyKey}:reload`);
  }

  function compare(original: GuidanceIntent) {
    void app.run(async () => {
      const bundle = await readGuidance(workspaceId);
      comparison = JSON.parse(original.body).scope.kind === 'global' ? bundle.global : bundle.workspace;
      notice('Current guidance loaded for comparison. Your original draft and operation are unchanged.');
    }, `${busyKey}:compare`);
  }

  function forget() {
    void app.run(async () => {
      if (!await confirmAction({
        title: 'Forget this save attempt?',
        body: 'It may already have completed or may still complete later. Forgetting does not cancel it. Copy your draft first.',
        confirm: 'Forget locally',
        destructive: true,
      })) return;
      await app.verify();
      app.pending = null;
      intent = null;
      failure = '';
      conflict = false;
      comparison = null;
      forgotten = true;
      edited = true;
      app.setDraft(JSON.stringify({scope: board.scope, base_revision: board.revision, text}, null, 2));
    }, `${busyKey}:forget`);
  }
</script>

{#snippet metadata(source: Board)}
  <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
    <Badge tone="outline">{source.revision === 0 ? 'Not yet published' : `Revision ${source.revision}`}</Badge>
    {#if source.publisher}
      <span>{stamp(source.publisher.recorded_at)} · {source.publisher.actor_kind} <span class="font-mono">{source.publisher.actor_id}</span> · Principal <span class="font-mono">{source.publisher.principal_id}</span></span>
    {/if}
  </div>
{/snippet}

<Card>
  <CardHeader title={global ? 'Global board' : 'Workspace board'} description={note}>
    {#snippet action()}
      {#if editable}<Badge tone="info">Editable</Badge>{:else}<Badge>Read-only</Badge>{/if}
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    {@render metadata(board)}

    {#if intent}
      {@const original = intent}
      <Alert tone="warning" title={conflict ? 'Guidance save conflict' : 'Guidance save not confirmed'} live>
        <div class="space-y-3">
          {#if failure}<p>{failure}</p>{/if}
          {#if conflict}
            <p>The board may have moved past revision {board.revision}, or this exact save may already have been accepted. Compare the current board before you decide.</p>
          {/if}
          <p>The editor keeps the original draft, operation ID and base revision. Nothing retries automatically, and forgetting locally doesn't cancel server execution. Copy your draft before you reload and reconcile it with the current board.</p>
          <Field id={`${id}-original`} label="Original guidance save">
            <Textarea id={`${id}-original`} readonly rows={5} value={original.body} class="font-mono text-xs text-foreground" />
          </Field>
          <div class="flex flex-wrap gap-2">
            <Button size="sm" busy={app.working(`${busyKey}:retry`)} onclick={() => app.run(() => send(original), `${busyKey}:retry`)}>Check or retry exact save</Button>
            <Button size="sm" variant="outline" onclick={() => copyText(JSON.parse(original.body).text)}><Copy /> Copy draft</Button>
            <Button size="sm" variant="outline" busy={app.working(`${busyKey}:compare`)} onclick={() => compare(original)}><GitCompare /> Compare current guidance</Button>
            <Button size="sm" variant="ghost" busy={app.working(`${busyKey}:forget`)} onclick={forget}>Forget local attempt</Button>
          </div>
          {#if comparison}
            <div class="space-y-2 rounded-md border bg-background p-3">
              <h3 class="text-sm font-medium text-foreground">Current server board</h3>
              {@render metadata(comparison)}
              <pre class="max-h-64 overflow-auto font-mono text-xs whitespace-pre-wrap break-words text-foreground">{comparison.text || '(Empty guidance)'}</pre>
            </div>
          {/if}
        </div>
      </Alert>
    {:else if forgotten}
      <Alert title="Local attempt forgotten" live>
        <p>The editor still uses its original base revision. Copy or merge your draft, reload current guidance, then paste and review before a new save.</p>
      </Alert>
    {/if}

    {#if confirmedRevision !== null}
      <Alert tone="success" title={`Save confirmed at revision ${confirmedRevision}`} live>
        <p>Reload to read the current server board.</p>
      </Alert>
    {/if}

    <form class="space-y-4" onsubmit={save}>
      <Field {id} label={global ? 'Global instructions' : 'Workspace instructions'}
        hint={`${size} / ${MAX_BYTES} UTF-8 bytes`}
        error={over ? `${size} / ${MAX_BYTES} UTF-8 bytes. Shorten the text before saving.` : undefined}>
        <Textarea {id} bind:value={text} rows={14} readonly={!editable || intent !== null} oninput={input}
          aria-invalid={over || undefined} aria-describedby={over ? `${id}-error` : `${id}-hint`}
          class="min-h-72 font-mono text-[13px] leading-relaxed" />
      </Field>
      {#if editable}
        <div class="flex flex-wrap gap-2">
          <Button type="submit" busy={app.working(`${busyKey}:save`)} disabled={!edited || over || app.pending !== null}><Save /> Save guidance</Button>
          <Button variant="outline" busy={app.working(`${busyKey}:reload`)} onclick={reload}><RotateCcw /> Discard edits and reload</Button>
        </div>
      {/if}
    </form>
  </CardContent>
</Card>
