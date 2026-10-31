<script lang="ts">
  // Permanent deletion of an archived workspace: server preview, typed UUID
  // and explicit confirmation, then a start request carrying an operation ID
  // and the previewed lifecycle revision. Instance keys can also look up an
  // earlier deletion by its IDs.
  import {Trash2} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Separator from '../../lib/ui/Separator.svelte';
  import {app} from '../../lib/app.svelte';
  import {canChangeLifecycle, canDeleteWorkspace} from '../../lib/model.js';
  import type {Session, Workspace} from '../../lib/types';
  import DeletionLookup from './DeletionLookup.svelte';
  import type {SettingsState} from './state.svelte';

  let {ctl, workspace, session}: {ctl: SettingsState; workspace: Workspace; session: Session} = $props();

  const allowed = $derived(canChangeLifecycle(session, workspace));
  const deletable = $derived(canDeleteWorkspace(session, workspace));
  let typed: HTMLInputElement | null = $state(null);
  $effect(() => { if (ctl.preview && typed) typed.focus(); });

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(() => ctl.submitDeletion(), 'deletion-start');
  }
</script>

<Card class="border-destructive/40">
  <CardHeader title="Danger zone" description="Irreversible actions for this workspace." />
  <CardContent class="space-y-4">
    {#if allowed}
      <div class="space-y-3">
        <h3 class="text-sm font-medium">Permanently delete workspace</h3>
        {#if ctl.preview}
          {@const preview = ctl.preview}
          <div class="space-y-3 text-sm">
            <p>Delete <span class="font-medium">{preview.name}</span> (<code class="font-mono text-[13px] break-all">{preview.workspace_id}</code>)? All access ends immediately when deletion starts.</p>
            <p class="text-muted-foreground">Shared client identities and access to other workspaces remain. Minimal deletion and security records remain. Large cleanups can take hours; “started” does not mean “complete”.</p>
          </div>
          <Alert tone="destructive" title="Save the receipt">
            Workspace keys and their browser sessions lose access at start. A current instance administrator can check completion or investigate a lost response.
          </Alert>
          <form class="space-y-4" onsubmit={submit} oninput={() => ctl.deletionDraft()}>
            <Field id="deletion-confirm-id" label="Type the exact workspace UUID">
              <Input id="deletion-confirm-id" bind:ref={typed} required autocomplete="off" spellcheck={false}
                class="font-mono" bind:value={ctl.deletion.typed} />
            </Field>
            <label class="flex items-start gap-3 text-sm">
              <input type="checkbox" required class="mt-0.5 size-4 shrink-0 accent-destructive" bind:checked={ctl.deletion.confirmed} />
              <span>I confirm permanent deletion of this archived workspace and its contents and history.</span>
            </label>
            <div class="flex flex-wrap gap-2">
              <Button type="submit" variant="destructive" busy={app.working('deletion-start')}><Trash2 /> Permanently delete workspace</Button>
              <Button variant="ghost" busy={app.working('deletion-cancel')}
                onclick={() => app.run(() => ctl.cancelDeletion(), 'deletion-cancel')}>Cancel deletion preparation</Button>
            </div>
          </form>
        {:else}
          <p class="text-sm text-muted-foreground">
            Remove this workspace’s entries, archives, revision history, guidance and status messages. This cannot be undone. Backup retention is separate.
          </p>
          {#if deletable}
            <Button variant="destructive" disabled={ctl.frozen} busy={app.working('deletion-prepare')}
              onclick={() => app.run(() => ctl.prepareDeletion(), 'deletion-prepare')}><Trash2 /> Prepare permanent deletion</Button>
          {:else}
            <p class="text-sm text-muted-foreground">Archive this workspace first. Archiving preserves readable context and can be reversed.</p>
          {/if}
        {/if}
      </div>
    {/if}
    {#if session.identity.scope.kind === 'instance' && !ctl.preview}
      {#if allowed}<Separator />{/if}
      <DeletionLookup />
    {/if}
  </CardContent>
</Card>
