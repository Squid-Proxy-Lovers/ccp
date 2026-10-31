<script lang="ts">
  import {tick} from 'svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import {parseEnrollmentLink, enrollmentCommand} from '../lib/enrollment_link.js';
  let {value, ondismiss}: {value: string; ondismiss?: () => void} = $props();
  let shown = $state(false);
  let command = $state(false);
  let message = $state('');
  let field: HTMLInputElement | null = $state(null);
  const displayed = $derived(command ? enrollmentCommand(parseEnrollmentLink(value)) : value);
  async function copy(asCommand = false) {
    command = asCommand;
    try {
      await navigator.clipboard.writeText(asCommand ? enrollmentCommand(parseEnrollmentLink(value)) : value);
      message = asCommand ? 'POSIX command copied. It includes the secret link.' : 'Enrollment link copied. Share it privately.';
    } catch {
      shown = true;
      message = 'Clipboard access failed. The value is shown and selected; copy it manually.';
      await tick(); field?.focus(); field?.select();
    }
  }
</script>

<section aria-label="Private enrollment link" class="grid gap-4 rounded-lg border border-warning/35 bg-warning/[0.06] p-4">
  <div class="space-y-1">
    <h3 class="text-base font-semibold">Save the enrollment link privately</h3>
    <p class="text-sm text-muted-foreground">Anyone holding this link can use its remaining admissions until it expires or is revoked. The server cannot show it again.</p>
  </div>
  <div class="space-y-2">
    <label for="enrollment-link-value" class="text-sm font-medium">{command ? 'POSIX enrollment command' : 'Enrollment link'}</label>
    <Input id="enrollment-link-value" bind:ref={field} type={shown ? 'text' : 'password'} readonly value={displayed} autocomplete="off" spellcheck="false" class="font-mono text-xs" />
    <div class="flex flex-wrap gap-2">
      <Button variant="outline" onclick={() => void copy()}>Copy enrollment link</Button>
      <Button variant="ghost" aria-pressed={shown} onclick={() => shown = !shown}>{shown ? 'Hide' : 'Show'} {command ? 'command' : 'link'}</Button>
    </div>
  </div>
  <div class="space-y-2 text-sm">
    <p>Run <code class="rounded border bg-background px-1.5 py-0.5 font-mono">ccp enroll</code>, then paste the link at the hidden prompt.</p>
    <details class="space-y-2">
      <summary class="cursor-pointer rounded outline-none focus-visible:ring-2 focus-visible:ring-ring">Optional command for sh, bash or zsh</summary>
      <p class="pt-2 text-muted-foreground">This includes the secret in command arguments, which may be saved in shell history or visible in process listings. Use hidden paste in other shells.</p>
      <Button variant="outline" size="sm" onclick={() => void copy(true)}>Copy POSIX enrollment command</Button>
    </details>
  </div>
  {#if message}<p role="status" class="text-sm">{message}</p>{/if}
  {#if ondismiss}<div><Button variant="ghost" size="sm" onclick={ondismiss}>I've saved the link, dismiss</Button></div>{/if}
</section>
