<script lang="ts">
  // Shelf or book creation. The draft lives in app.formDraft so it survives
  // navigation and appears in session-lock recovery; it is dropped only once
  // the server acknowledges the create.
  import {untrack} from 'svelte';
  import {Plus} from '@lucide/svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import {app} from '../../lib/app.svelte';

  let {kind, draftKey, oncreate}: {
    kind: 'shelf' | 'book'; draftKey: string;
    /** Resolves true once the server acknowledged the create. */
    oncreate: (name: string, description: string) => Promise<boolean>;
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

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (!(await oncreate(draft.name ?? '', draft.description ?? ''))) return;
      app.dropFormDraft(draftKey);
      draft = acquire();
      open = false;
    }, key);
  }
</script>

<details bind:open class="group">
  <summary class="flex cursor-pointer list-none items-center gap-1.5 rounded-md text-sm font-medium outline-none select-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden">
    <Plus class="size-4 text-muted-foreground transition-transform group-open:rotate-45" aria-hidden="true" />
    New {kind}
    {#if !open && (draft.name || draft.description)}<span class="text-xs font-normal text-muted-foreground">(draft kept)</span>{/if}
  </summary>
  <form class="mt-3 grid gap-3" onsubmit={submit}>
    <Field id={`create-${kind}-name`} label="Name">
      <Input id={`create-${kind}-name`} bind:value={draft.name} required autocomplete="off" />
    </Field>
    <Field id={`create-${kind}-description`} label="Description">
      <Input id={`create-${kind}-description`} bind:value={draft.description} autocomplete="off" />
    </Field>
    <div class="flex justify-end">
      <Button type="submit" size="sm" busy={app.working(key)}>Create {kind}</Button>
    </div>
  </form>
</details>
