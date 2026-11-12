<script lang="ts">
  // Shelf or book creation. The draft lives in app.formDraft so it survives
  // navigation and appears in session-lock recovery; it is dropped only once
  // the server acknowledges the create.
  import {tick, untrack} from 'svelte';
  import {Plus} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import {app as panelApp} from '../../lib/app.svelte';
  import {mountedView} from '../../lib/viewLifecycle.js';
  import {hasDraftContent} from '../../lib/drafts.js';
  import {discardDraft} from '../../lib/draftDiscard.js';
  import {formFrozen} from '../../lib/identitySwitch.js';

  const app = mountedView(panelApp);
  const {notice, confirmAction} = app;

  let {kind, draftKey, oncreate}: {
    kind: 'shelf' | 'book'; draftKey: string;
    /** Resolves true once the server acknowledged the create. */
    oncreate: (draft: Record<string, string>) => Promise<boolean>;
  } = $props();

  function acquire(): Record<string, string> {
    const draft = app.formDraft(untrack(() => draftKey));
    draft.name ??= '';
    draft.description ??= '';
    return draft;
  }
  const initial = acquire();
  let draft = $state.raw(initial);
  // Reopen the form when returning to a kept draft.
  let open = $state(Boolean(initial.name || initial.description));
  const key = $derived(`create-${kind}`);
  const frozen = $derived(formFrozen(app.busy, app.busyKey, key));

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const submitted = draft;
    void app.run(async () => {
      if (!(await oncreate(submitted))) return;
      draft = acquire();
      open = false;
    }, key);
  }

  async function discard() {
    const current = draft;
    const fresh = await discardDraft({
      drafts: app.drafts, key: draftKey, current,
      isBlocked: () => formFrozen(app.busy, app.busyKey, key),
      confirm: () => confirmAction({title: 'Discard draft?', body: `Discard this local ${kind} draft?`, confirm: 'Discard draft', destructive: true}),
    });
    if (!fresh) return;
    draft = fresh;
    notice(`Local ${kind} draft discarded.`);
    await tick();
    app.effect(() => document.getElementById(`create-${kind}-name`)?.focus());
  }
</script>

<details bind:open class="group">
  <summary class="flex cursor-pointer list-none items-center gap-1.5 rounded-md text-sm font-medium outline-none select-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
    <Plus class="size-4 text-muted-foreground transition-transform group-open:rotate-45" aria-hidden="true" />
    New {kind}
    {#if !open && (draft.name || draft.description)}<span class="text-xs font-normal text-muted-foreground">(draft kept)</span>{/if}
  </summary>
  <form class="mt-3 grid gap-3" onsubmit={submit}>
    <Field id={`create-${kind}-name`} label="Name">
      <Input id={`create-${kind}-name`} bind:value={draft.name} required autocomplete="off" disabled={frozen} />
    </Field>
    <Field id={`create-${kind}-description`} label="Description">
      <Input id={`create-${kind}-description`} bind:value={draft.description} autocomplete="off" disabled={frozen} />
    </Field>
    <div class="flex justify-end gap-2">
      <Button type="submit" size="sm" busy={app.working(key)} disabled={frozen}>Create {kind}</Button>
      {#if hasDraftContent(draft)}<Button type="button" variant="ghost" size="sm" onclick={discard} disabled={frozen}>Discard draft</Button>{/if}
    </div>
  </form>
</details>
