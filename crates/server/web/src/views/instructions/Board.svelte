<script lang="ts">
  // One instruction board: the global board or a session's board. Saving
  // replaces the whole text and goes live for agents immediately.
  import {Copy, RotateCcw, Save} from '@lucide/svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {app} from '../../lib/app.svelte';
  import {seconds, utf8Bytes} from '../../lib/api';
  import {copyText, notice, relative, stamp} from '../../lib/notify.svelte';
  import type {Instructions} from '../../lib/types';

  let {title, description, path, record, editable = true, onsaved}: {
    title: string; description: string; path: string; record: Instructions; editable?: boolean; onsaved?: (next: Instructions) => void;
  } = $props();

  // The editor starts from the loaded record; a new record remounts this component.
  // svelte-ignore state_referenced_locally
  let text = $state(record.content);
  // svelte-ignore state_referenced_locally
  let base = $state.raw<Instructions>(record);
  const edited = $derived(text !== base.content);
  const id = $derived(`board-${path.replace(/[^a-z0-9]+/gi, '-')}`);
  const updated = $derived(seconds(base.updated_at));
  const busyKey = $derived(`board:${path}`);

  $effect(() => { if (editable) app.dirty = edited; });
  $effect(() => () => { if (editable) app.dirty = false; });

  function save(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (!text.trim() && !await confirmAction({
        title: 'Clear these instructions?',
        body: 'Saving empty text removes these master instructions for every agent. It goes live immediately.',
        confirm: 'Save empty instructions',
        destructive: true,
      })) return;
      // Detect a concurrent edit before overwriting it.
      const current = await app.request<Instructions>(path);
      if (current.content !== base.content && !await confirmAction({
        title: 'Instructions changed on the server',
        body: `Someone saved a different version ${relative(seconds(current.updated_at))}. Saving now replaces their text with yours.`,
        confirm: 'Replace with my version',
        destructive: true,
      })) return;
      const next = await app.request<Instructions>(path, {method: 'PUT', body: {content: text}});
      base = next;
      text = next.content;
      onsaved?.(next);
      notice(`${title} saved. Agents see the new text on their next CCP call.`, 'success');
    }, `${busyKey}:save`);
  }

  function reload() {
    void app.run(async () => {
      if (edited && !await confirmAction({
        title: 'Discard your edits?',
        body: 'This reloads the saved instructions and drops your changes. Copy anything you want to keep first.',
        confirm: 'Discard and reload',
        destructive: true,
      })) return;
      const next = await app.request<Instructions>(path);
      base = next;
      text = next.content;
      notice('Loaded the current instructions.');
    }, `${busyKey}:reload`);
  }
</script>

<Card>
  <CardHeader {title} {description}>
    {#snippet action()}
      {#if !editable}<Badge>Read-only here</Badge>{:else if edited}<Badge tone="warning">Unsaved</Badge>{:else if base.content}<Badge tone="success">Live</Badge>{:else}<Badge>Empty</Badge>{/if}
    {/snippet}
  </CardHeader>
  <CardContent class="space-y-4">
    <p class="text-xs text-muted-foreground">
      {#if updated}Last saved <time datetime={new Date(updated * 1000).toISOString()} title={stamp(updated)}>{relative(updated)}</time>{:else}Never saved{/if}
    </p>
    <form class="space-y-4" onsubmit={save}>
      <Field {id} label={title} hint={`${utf8Bytes(text).toLocaleString()} bytes · ${text ? text.split('\n').length : 0} lines`}>
        <Textarea {id} bind:value={text} rows={editable ? 18 : 10} readonly={!editable}
          placeholder={editable ? 'Write instructions every agent should follow…' : '(No global instructions)'}
          class="min-h-56 font-mono text-[13px] leading-relaxed" />
      </Field>
      <div class="flex flex-wrap gap-2">
        {#if editable}
          <Button type="submit" busy={app.working(`${busyKey}:save`)} disabled={!edited}><Save /> Save and publish</Button>
          <Button variant="outline" busy={app.working(`${busyKey}:reload`)} onclick={reload}><RotateCcw /> {edited ? 'Discard edits' : 'Reload'}</Button>
        {/if}
        <Button variant="ghost" disabled={!text} onclick={() => copyText(text)}><Copy /> Copy</Button>
      </div>
    </form>
  </CardContent>
</Card>
