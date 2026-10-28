<script lang="ts">
  // Instance keys only: read the status of an earlier deletion by its IDs.
  // This never starts or repeats a deletion. Results appear in DeletionTracker.
  import {Search} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import {app} from '../../lib/app.svelte';
  import {lookupDeletion} from './deletion';

  const fields = app.formDraft('deletion-lookup');
  fields.workspace ??= '';
  fields.operation ??= '';

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      await lookupDeletion((fields.workspace ?? '').trim(), (fields.operation ?? '').trim());
      if (app.deletion) app.dropFormDraft('deletion-lookup');
    }, 'deletion-lookup');
  }
</script>

<div class="space-y-3">
  <div class="space-y-1">
    <h3 class="text-sm font-medium">Inspect a workspace deletion</h3>
    <p class="text-sm text-muted-foreground">
      Use the workspace and operation UUIDs from the original deletion request. This only reads status; it does not start or repeat deletion.
    </p>
  </div>
  {#if app.deletion}
    <p class="text-sm text-muted-foreground">Deletion tracking is already open on this page. Copy or forget it before looking up another deletion.</p>
  {:else}
    <form class="space-y-4" onsubmit={submit}>
      <div class="grid gap-4 md:grid-cols-2">
        <Field id="deletion-lookup-workspace" label="Deleted workspace UUID">
          <Input id="deletion-lookup-workspace" required autocomplete="off" spellcheck={false} class="font-mono" bind:value={fields.workspace} />
        </Field>
        <Field id="deletion-lookup-operation" label="Deletion operation UUID">
          <Input id="deletion-lookup-operation" required autocomplete="off" spellcheck={false} class="font-mono" bind:value={fields.operation} />
        </Field>
      </div>
      <Button type="submit" variant="outline" busy={app.working('deletion-lookup')}><Search /> Inspect deletion</Button>
    </form>
  {/if}
</div>
