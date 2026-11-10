<script lang="ts">
  import {tick} from 'svelte';
  import Secret from '../lib/ui/Secret.svelte';
  import EnrollmentLinkCard from './EnrollmentLinkCard.svelte';
  import {app} from '../lib/app.svelte';
  import type {AccessSecret} from '../lib/types';
  let {class: className = ''}: {class?: string} = $props();
  async function dismiss(secret: AccessSecret) {
    if (app.accessSecret !== secret) return;
    const scope = app.forSession();
    app.accessSecret = null;
    await tick();
    scope.effect(() => document.getElementById('page-title')?.focus());
  }
</script>

{#if app.accessSecret}
  {@const secret = app.accessSecret}
  <div class={className}>
    {#if secret.kind === 'enrollment_link'}
      <EnrollmentLinkCard value={secret.value} ondismiss={() => void dismiss(secret)} />
    {:else}
      <Secret title={secret.title} value={secret.value} ondismiss={() => void dismiss(secret)} />
    {/if}
  </div>
{/if}
