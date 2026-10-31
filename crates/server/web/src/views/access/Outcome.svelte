<script lang="ts">
  // Replaces the access sections after a write: an unconfirmed outcome with its
  // original request, a local copy after "forget", or a confirmed acknowledgement.
  // While shown, no other access control is available (the old panel froze them).
  import {tick} from 'svelte';
  import {Copy} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Secret from '../../lib/ui/Secret.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText, notice} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import type {Outcome} from './actions';

  let {outcome = $bindable(), onreread}: {outcome: Outcome; onreread: () => Promise<void>} = $props();
  let heading: HTMLHeadingElement | undefined = $state();

  // Controls the user activated have just been removed; land on the new heading.
  const phase = $derived(outcome.phase);
  $effect(() => {
    void phase;
    void tick().then(() => heading?.focus());
  });

  function observe() {
    const current = outcome;
    if (current.phase !== 'pending') return;
    void app.run(async () => {
      const observation = await current.observe();
      if (outcome === current) outcome = {...current, observation};
    }, 'access-observe');
  }

  async function forget() {
    if (outcome.phase !== 'pending') return;
    const {wording, attempt} = outcome;
    if (!await confirmAction({title: 'Forget this action locally?', body: wording.forgetConfirm, confirm: 'Forget locally', destructive: true})) return;
    app.pending = null;
    app.setDraft(attempt.body);
    outcome = {phase: 'local', wording, attempt};
  }

  async function discard() {
    if (outcome.phase !== 'local') return;
    const {wording} = outcome;
    if (!await confirmAction({title: 'Discard local copy?', body: wording.discardConfirm, confirm: 'Discard copy', destructive: true})) return;
    app.clean();
    outcome = {phase: 'discarded', title: wording.discardedTitle, body: wording.discardedBody, reread: wording.reread};
    if (wording.discardedNotice) notice(wording.discardedNotice, 'warning');
  }

  const reread = () => app.run(onreread, 'access-reread');
</script>

<section aria-labelledby="access-outcome-title"
  class={['space-y-4 rounded-xl border p-5 shadow-xs', outcome.phase === 'pending' || outcome.phase === 'local' ? 'border-warning/35 bg-warning/[0.04]' : 'bg-card']}>
  {#if outcome.phase === 'pending'}
    <h2 bind:this={heading} id="access-outcome-title" tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{outcome.wording.pendingTitle}</h2>
    <Alert tone="destructive" title={outcome.error} />
    <p class="text-sm text-muted-foreground">{outcome.wording.pendingBody}</p>
    <Field id="access-original" label={outcome.wording.original}>
      <Textarea id="access-original" readonly value={outcome.attempt.body} rows={6} class="font-mono text-xs" />
    </Field>
    <div class="flex flex-wrap gap-2">
      <Button variant="outline" onclick={() => copyText(outcome.phase === 'pending' ? outcome.attempt.body : '')}><Copy /> {outcome.wording.copyOriginal}</Button>
      <Button variant="outline" busy={app.working('access-observe')} onclick={observe}>{outcome.wording.observe}</Button>
      <Button variant="ghost" class="text-destructive hover:text-destructive" onclick={forget}>{outcome.wording.forget}</Button>
    </div>
    {#if outcome.observation}
      <div class="space-y-2" role="status">
        <p class="text-sm text-muted-foreground">{outcome.observation.note}</p>
        {#if outcome.observation.text}
          <pre class="max-h-80 overflow-auto rounded-md border bg-muted/50 p-3 font-mono text-xs">{outcome.observation.text}</pre>
        {/if}
      </div>
    {/if}
  {:else if outcome.phase === 'local'}
    <h2 bind:this={heading} id="access-outcome-title" tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{outcome.wording.localTitle}</h2>
    <p class="text-sm text-muted-foreground">{outcome.wording.localBody}</p>
    <Field id="access-local-copy" label={outcome.wording.localLabel}>
      <Textarea id="access-local-copy" readonly value={outcome.attempt.body} rows={6} class="font-mono text-xs" />
    </Field>
    <div class="flex flex-wrap gap-2">
      <Button variant="outline" onclick={() => copyText(outcome.phase === 'local' ? outcome.attempt.body : '')}><Copy /> {outcome.wording.copyLocal}</Button>
      {#if outcome.wording.rereadLocal}
        <Button variant="outline" busy={app.working('access-reread')} onclick={reread}>{outcome.wording.rereadLocal}</Button>
      {/if}
      <Button variant="ghost" class="text-destructive hover:text-destructive" onclick={discard}>{outcome.wording.discardLocal}</Button>
    </div>
  {:else if outcome.phase === 'done'}
    <h2 bind:this={heading} id="access-outcome-title" tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{outcome.title}</h2>
    <p class="text-sm">{outcome.message}</p>
    {#if outcome.secret}
      <Secret title={outcome.secret.title} value={outcome.secret.value}
        ondismiss={() => { if (outcome.phase === 'done') outcome = {...outcome, secret: undefined}; }} />
    {/if}
    {#if outcome.note}<p class="text-sm text-muted-foreground">{outcome.note}</p>{/if}
    <Button busy={app.working('access-reread')} onclick={reread}>{outcome.reread}</Button>
  {:else}
    <h2 bind:this={heading} id="access-outcome-title" tabindex="-1" class="text-[15px] font-semibold tracking-tight outline-none">{outcome.title}</h2>
    <p class="text-sm text-muted-foreground">{outcome.body}</p>
    <Button busy={app.working('access-reread')} onclick={reread}>{outcome.reread}</Button>
  {/if}
</section>
