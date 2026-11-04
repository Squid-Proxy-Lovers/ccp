<script lang="ts">
  // A one-time enrollment link. `issued` frames it for the admin who just created
  // it; `join` is the recipient's step on the public join page. The link itself
  // stays masked; the safe default is `ccp enroll` plus a hidden paste.
  import {tick} from 'svelte';
  import {ShieldAlert} from '@lucide/svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import SecretField from '../lib/ui/SecretField.svelte';
  import Command from '../lib/ui/Command.svelte';
  import {parseEnrollmentLink, enrollmentCommand} from '../lib/enrollment_link.js';
  let {value, ondismiss, variant = 'issued'}: {value: string; ondismiss?: () => void; variant?: 'issued' | 'join'} = $props();
  let message = $state('');
  let posix = $state('');
  let posixField = $state<HTMLInputElement | null>(null);
  // The origin and path are public; only the fragment carries the invitation.
  const prefix = $derived(value.slice(0, value.indexOf('#') + 1));

  function copied(ok: boolean) {
    message = ok ? (variant === 'join' ? 'Link copied. Paste it at the ccp enroll prompt.' : 'Enrollment link copied. Share it privately.')
      : 'Clipboard access failed. The link is shown and selected; copy it manually.';
  }
  async function copyCommand() {
    const command = enrollmentCommand(parseEnrollmentLink(value));
    try {
      await navigator.clipboard.writeText(command);
      posix = '';
      message = 'Command copied. It contains the secret link.';
    } catch {
      posix = command;
      message = 'Clipboard access failed. The command is shown and selected; copy it manually.';
      await tick();
      posixField?.focus();
      posixField?.select();
    }
  }
</script>

<section aria-label="Private enrollment link" class={['grid gap-4', variant === 'issued' && 'rounded-lg border bg-card p-4']}>
  {#if variant === 'issued'}
    <div class="flex gap-2.5">
      <ShieldAlert class="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
      <div class="space-y-0.5">
        <h3 class="text-sm font-semibold">Enrollment link · shown once</h3>
        <p class="text-sm text-muted-foreground">Anyone with this link can join until it expires or you revoke it. Share it privately.</p>
      </div>
    </div>
    <SecretField id="enrollment-link-value" label="Enrollment link" {prefix} {value} oncopy={copied} />
    <div class="grid gap-1.5">
      <p class="text-sm text-muted-foreground">The recipient runs this and pastes the link at the hidden prompt:</p>
      <Command command="ccp enroll" />
    </div>
  {:else}
    <Command command="ccp enroll" />
    <SecretField id="enrollment-link-value" label="Your enrollment link" {prefix} {value} oncopy={copied}
      hint="Paste it at the hidden prompt, so it stays out of your shell history." />
  {/if}
  <details class="group text-sm">
    <summary class="cursor-pointer rounded text-muted-foreground outline-none select-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">One-line command for sh, bash or zsh</summary>
    <div class="mt-2 grid gap-2 border-l-2 pl-3">
      <p class="text-muted-foreground">This puts the secret in the command itself, where shell history and process listings can see it. Prefer the hidden prompt, or for very long links save the link in a private file and pipe it to <code class="font-mono text-foreground">ccp enroll -</code>.</p>
      <div><Button variant="outline" size="sm" onclick={() => void copyCommand()}>Copy one-line command</Button></div>
      {#if posix}<Input id="enrollment-link-command" bind:ref={posixField} readonly value={posix} aria-label="One-line enrollment command" class="font-mono text-xs" />{/if}
    </div>
  </details>
  <p role="status" class="text-sm text-muted-foreground empty:hidden">{message}</p>
  {#if ondismiss && variant === 'join'}
    <button type="button" onclick={ondismiss}
      class="justify-self-start rounded text-xs text-muted-foreground underline-offset-4 outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring">Remove the link from this page</button>
  {:else if ondismiss}
    <div><Button variant="ghost" size="sm" class="-ml-2" onclick={ondismiss}>I've saved the link, dismiss</Button></div>
  {/if}
</section>
